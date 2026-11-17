//! Reading and writing goals, tasks, links and risks.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use fluxo_core::access::{self, Access, Graph, Role};
use sqlx::SqlitePool;

use crate::{Result, now};

pub type Id = i64;

#[derive(Clone, Debug)]
pub struct Node {
    pub id: Id,
    pub kind: String,
    pub title: String,
    pub what: String,
    pub why: String,
    pub owner_id: Option<Id>,
    pub home_parent_id: Option<Id>,
    pub coverage: Option<String>,
    pub clarity: Option<String>,
    pub estimate_days: Option<f64>,
    pub remaining_days: Option<f64>,
    pub status: String,
    pub started_at: Option<i64>,
    pub achieved_at: Option<i64>,
    pub private: bool,
    pub statement_changed_at: i64,
    pub created_at: i64,
    pub archived_at: Option<i64>,
    /// Goals not yet broken down: a rough size in working days.
    pub size: Option<(f64, f64)>,
}

impl Node {
    pub fn is_goal(&self) -> bool {
        self.kind == "goal"
    }
    pub fn is_done(&self) -> bool {
        if self.is_goal() { self.achieved_at.is_some() } else { self.status == "done" }
    }
    pub fn href(&self) -> String {
        if self.is_goal() { format!("/goals/{}", self.id) } else { format!("/tasks/{}", self.id) }
    }
}

macro_rules! node_from {
    ($r:expr) => {
        Node {
            id: $r.id,
            kind: $r.kind,
            title: $r.title,
            what: $r.what,
            why: $r.why,
            owner_id: $r.owner_id,
            home_parent_id: $r.home_parent_id,
            coverage: $r.coverage,
            clarity: $r.clarity,
            estimate_days: $r.estimate_days,
            remaining_days: $r.remaining_days,
            status: $r.status,
            started_at: $r.started_at,
            achieved_at: $r.achieved_at,
            private: $r.private != 0,
            statement_changed_at: $r.statement_changed_at,
            created_at: $r.created_at,
            archived_at: $r.archived_at,
            size: $r.size_min_days.zip($r.size_max_days),
        }
    };
}

/// Every node, archived ones included, keyed by id. Fine at the sizes fluxo expects per instance;
/// revisit with indexes and partial loads if instances grow past tens of thousands of nodes.
pub async fn all_nodes(db: &SqlitePool) -> Result<HashMap<Id, Node>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", kind, title, what, why, owner_id, home_parent_id, coverage, clarity, estimate_days,
                  remaining_days, status, started_at, achieved_at, private, statement_changed_at, created_at, archived_at,
                  size_min_days, size_max_days
           FROM nodes"#
    )
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| (r.id, node_from!(r))).collect())
}

pub async fn parent_links(db: &SqlitePool) -> Result<Vec<(Id, Id)>> {
    Ok(sqlx::query!("SELECT child_id, parent_id FROM node_parents")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.child_id, r.parent_id))
        .collect())
}

pub async fn need_links(db: &SqlitePool) -> Result<Vec<(Id, Id)>> {
    Ok(sqlx::query!("SELECT node_id, needs_id FROM dependencies")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.node_id, r.needs_id))
        .collect())
}

/// The whole picture a page needs: nodes, links, and access rules.
pub struct World {
    pub nodes: HashMap<Id, Node>,
    pub parents: HashMap<Id, Vec<Id>>,
    pub children: HashMap<Id, Vec<Id>>,
    pub needs: HashMap<Id, Vec<Id>>,
    pub needed_by: HashMap<Id, Vec<Id>>,
    pub graph: Graph,
    /// Every private item, worked out once per load.
    private: HashSet<Id>,
    /// Each person's role on every item, worked out once per load when first needed, so checking
    /// every item on a page stays linear however deep the goals go.
    roles: Mutex<HashMap<Id, Arc<HashMap<Id, Access>>>>,
}

impl World {
    pub async fn load(db: &SqlitePool) -> Result<World> {
        let nodes = all_nodes(db).await?;
        let mut parents: HashMap<Id, Vec<Id>> = HashMap::new();
        let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
        for (c, p) in parent_links(db).await? {
            parents.entry(c).or_default().push(p);
            children.entry(p).or_default().push(c);
        }
        for v in children.values_mut() {
            v.sort_by_key(|id| (nodes.get(id).map(|n| n.created_at).unwrap_or(0), *id));
        }
        let mut needs: HashMap<Id, Vec<Id>> = HashMap::new();
        let mut needed_by: HashMap<Id, Vec<Id>> = HashMap::new();
        for (n, d) in need_links(db).await? {
            needs.entry(n).or_default().push(d);
            needed_by.entry(d).or_default().push(n);
        }
        let roles = sqlx::query!("SELECT node_id, user_id, role FROM memberships WHERE status = 'active'")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|r| ((r.node_id, r.user_id), if r.role == "contributor" { Role::Contributor } else { Role::Watcher }))
            .collect();
        let graph = Graph {
            nodes: nodes
                .values()
                .map(|n| {
                    let ps = parents.get(&n.id).cloned().unwrap_or_default();
                    (
                        n.id,
                        access::Node {
                            owner: n.owner_id,
                            home: n.home_parent_id,
                            parents: ps,
                            private: n.private,
                            archived: n.archived_at.is_some(),
                        },
                    )
                })
                .collect(),
            roles,
        };
        let private = graph.private_set();
        Ok(World { nodes, parents, children, needs, needed_by, graph, private, roles: Mutex::default() })
    }

    /// The person's role on `id`, ignoring privacy and archiving (see [`Graph::role`]).
    pub fn role(&self, user: Id, id: Id) -> Access {
        self.roles_for(user).get(&id).copied().unwrap_or(Access::None)
    }

    pub fn access(&self, user: Id, id: Id) -> Access {
        self.graph.access_with(id, self.role(user, id), &self.private)
    }

    /// Private if it, or any goal on its home-parent chain, is marked private.
    pub fn is_private(&self, id: Id) -> bool {
        self.private.contains(&id)
    }

    fn roles_for(&self, user: Id) -> Arc<HashMap<Id, Access>> {
        let mut cache = self.roles.lock().unwrap_or_else(|e| e.into_inner());
        cache.entry(user).or_insert_with(|| Arc::new(self.graph.roles_for(user))).clone()
    }

    /// The title `user` may see for `id`: the real one if they can read it, else a placeholder.
    pub fn title_for(&self, user: Id, id: Id) -> String {
        match self.nodes.get(&id) {
            Some(n) if self.access(user, id).can_read() => n.title.clone(),
            Some(n) if n.is_goal() => "a private goal".into(),
            Some(_) => "a private step".into(),
            None => "something".into(),
        }
    }

    pub fn children_of(&self, id: Id) -> Vec<&Node> {
        self.children
            .get(&id)
            .map(|c| c.iter().filter_map(|i| self.nodes.get(i)).filter(|n| n.archived_at.is_none()).collect())
            .unwrap_or_default()
    }

    pub fn archived_children_of(&self, id: Id) -> Vec<&Node> {
        self.children
            .get(&id)
            .map(|c| c.iter().filter_map(|i| self.nodes.get(i)).filter(|n| n.archived_at.is_some()).collect())
            .unwrap_or_default()
    }

    pub fn parents_of(&self, id: Id) -> Vec<&Node> {
        self.parents.get(&id).map(|p| p.iter().filter_map(|i| self.nodes.get(i)).collect()).unwrap_or_default()
    }

    pub fn needs_of(&self, id: Id) -> &[Id] {
        self.needs.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn needed_by(&self, id: Id) -> &[Id] {
        self.needed_by.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Home-parent chain from the top down to (not including) `id`.
    pub fn home_chain(&self, id: Id) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cur = self.nodes.get(&id).and_then(|n| n.home_parent_id);
        while let Some(p) = cur {
            if !seen.insert(p) {
                break;
            }
            let Some(n) = self.nodes.get(&p) else { break };
            out.push(n);
            cur = n.home_parent_id;
        }
        out.reverse();
        out
    }

    /// Would these extra links make an order of work impossible (a loop)? Uses the simulation
    /// engine's own validation, so anything accepted here can be forecast.
    pub fn would_loop(&self, extra_parent: Option<(Id, Id)>, extra_need: Option<(Id, Id)>) -> bool {
        use fluxo_sim::{Clarity, Coverage, Model, Node as SimNode};
        let ids: Vec<Id> = self.nodes.keys().copied().collect();
        let index: HashMap<Id, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let mut m = Model::default();
        for id in &ids {
            let n = &self.nodes[id];
            m.add(if n.is_goal() { SimNode::goal(Coverage::Full) } else { SimNode::task(0.0, Clarity::High) });
        }
        let link_parent = |m: &mut Model, c: Id, p: Id| {
            if let (Some(&ci), Some(&pi)) = (index.get(&c), index.get(&p)) {
                m.feed(ci, pi);
            }
        };
        for (c, ps) in &self.parents {
            for p in ps {
                link_parent(&mut m, *c, *p);
            }
        }
        if let Some((c, p)) = extra_parent {
            link_parent(&mut m, c, p);
        }
        let link_need = |m: &mut Model, a: Id, b: Id| {
            if let (Some(&ai), Some(&bi)) = (index.get(&a), index.get(&b)) {
                m.need(ai, bi);
            }
        };
        for (a, ds) in &self.needs {
            for d in ds {
                link_need(&mut m, *a, *d);
            }
        }
        if let Some((a, b)) = extra_need {
            link_need(&mut m, a, b);
        }
        matches!(
            m.compile(),
            Err(fluxo_sim::ModelError::Cycle { .. }) | Err(fluxo_sim::ModelError::ParentIsNotAGoal { .. })
        )
    }
}

pub async fn usernames(db: &SqlitePool) -> Result<HashMap<Id, String>> {
    Ok(sqlx::query!(r#"SELECT id AS "id!", username FROM users"#)
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|r| (r.id, r.username))
        .collect())
}

pub async fn record(db: &SqlitePool, node: Id, actor: Id, kind: &str, detail: serde_json::Value) -> Result<()> {
    let (t, detail) = (now(), detail.to_string());
    sqlx::query!(
        "INSERT INTO events (node_id, actor_id, kind, detail, at) VALUES (?, ?, ?, ?, ?)",
        node,
        actor,
        kind,
        detail,
        t
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct RiskRow {
    pub id: Id,
    pub title: String,
    pub min: f64,
    pub max: f64,
    pub likelihood: i64,
    /// Likelihood history, oldest first.
    pub history: Vec<i64>,
}

pub async fn risks(db: &SqlitePool, node: Id) -> Result<Vec<RiskRow>> {
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!", r.title, r.impact_min_days, r.impact_max_days, l.likelihood_pct
           FROM risks r JOIN risk_likelihood l ON l.risk_id = r.id
           WHERE r.node_id = ? ORDER BY r.created_at, r.id, l.id"#,
        node
    )
    .fetch_all(db)
    .await?;
    let mut out: Vec<RiskRow> = Vec::new();
    for r in rows {
        match out.last_mut() {
            Some(last) if last.id == r.id => {
                last.history.push(r.likelihood_pct);
                last.likelihood = r.likelihood_pct;
            }
            _ => out.push(RiskRow {
                id: r.id,
                title: r.title,
                min: r.impact_min_days,
                max: r.impact_max_days,
                likelihood: r.likelihood_pct,
                history: vec![r.likelihood_pct],
            }),
        }
    }
    Ok(out)
}
