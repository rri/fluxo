//! Who can do what with a goal or task.
//!
//! - **Reading:** everything is visible to every signed-in person unless it's private. An item is
//!   private if it, or any goal on its **home-parent chain**, is marked private (decided
//!   2026-11-16). Private items are visible only to people with a role on them or above them.
//! - **Roles** flow down from **every** parent goal; the nearest one wins (D17). Owning an item, or
//!   any goal above it, makes you an owner of it.
//! - Archived items are visible only to their owners (so they can restore or delete them).

use std::collections::{HashMap, HashSet, VecDeque};

pub type Id = i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    None,
    Read,
    Watch,
    Contribute,
    Own,
}

impl Access {
    pub fn can_read(self) -> bool {
        self >= Access::Read
    }
    /// Change the path: add steps, estimates, risks, links, progress.
    pub fn can_contribute(self) -> bool {
        self >= Access::Contribute
    }
    /// Change the goal itself (statement, why, coverage, privacy), archive, delete, invite.
    pub fn is_owner(self) -> bool {
        self == Access::Own
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Contributor,
    Watcher,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub owner: Option<Id>,
    pub home: Option<Id>,
    pub parents: Vec<Id>,
    pub private: bool,
    pub archived: bool,
}

/// Everything access rules need, loaded once per request.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: HashMap<Id, Node>,
    /// Active memberships: (node, user) → role.
    pub roles: HashMap<(Id, Id), Role>,
}

impl Graph {
    /// Private if the node or any goal on its home-parent chain is marked private.
    pub fn is_private(&self, id: Id) -> bool {
        let mut seen = HashSet::new();
        let mut cur = Some(id);
        while let Some(i) = cur {
            if !seen.insert(i) {
                break;
            }
            let Some(n) = self.nodes.get(&i) else { break };
            if n.private {
                return true;
            }
            cur = n.home;
        }
        false
    }

    /// Breadth-first walk from `id` up through every parent, nearest first.
    fn upward(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
        let mut queue = VecDeque::from([id]);
        let mut seen = HashSet::from([id]);
        std::iter::from_fn(move || {
            let i = queue.pop_front()?;
            if let Some(n) = self.nodes.get(&i) {
                for &p in &n.parents {
                    if seen.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
            Some(i)
        })
    }

    /// The person's role on `id`, ignoring privacy and archiving: owner anywhere up the chain wins,
    /// otherwise the nearest membership (if two are equally near, Contributor beats Watcher).
    /// `Access::None` if they have no role. This walks everything above `id`; for many items at
    /// once, use [`Graph::roles_for`].
    pub fn role(&self, user: Id, id: Id) -> Access {
        let mut level = vec![id];
        let mut seen = HashSet::from([id]);
        let mut nearest: Option<Role> = None;
        while !level.is_empty() {
            for &i in &level {
                if self.nodes.get(&i).is_some_and(|n| n.owner == Some(user)) {
                    return Access::Own;
                }
            }
            if nearest.is_none() {
                nearest = level
                    .iter()
                    .filter_map(|&i| self.roles.get(&(i, user)).copied())
                    .max_by_key(|r| *r == Role::Contributor);
            }
            let mut next = Vec::new();
            for &i in &level {
                for &p in self.nodes.get(&i).map(|n| n.parents.as_slice()).unwrap_or(&[]) {
                    if seen.insert(p) {
                        next.push(p);
                    }
                }
            }
            level = next;
        }
        match nearest {
            Some(Role::Contributor) => Access::Contribute,
            Some(Role::Watcher) => Access::Watch,
            None => Access::None,
        }
    }

    /// [`Graph::role`] for every item at once, in one pass from the top down: linear in items
    /// and links, however deep or wide the goals are. Pages that check many items use this, so a
    /// very deep tree can't make every page slow.
    pub fn roles_for(&self, user: Id) -> HashMap<Id, Access> {
        // (owned at or above, nearest membership as (distance, role))
        type Memo = (bool, Option<(usize, Role)>);
        let mut memo: HashMap<Id, Memo> = HashMap::with_capacity(self.nodes.len());
        let mut on_stack: HashSet<Id> = HashSet::new();
        for &start in self.nodes.keys() {
            // Iterative depth-first walk up, so depth can't overflow the stack.
            let mut stack = vec![(start, false)];
            while let Some((id, expanded)) = stack.pop() {
                if memo.contains_key(&id) {
                    continue;
                }
                let parents = self.nodes.get(&id).map(|n| n.parents.as_slice()).unwrap_or(&[]);
                if !expanded {
                    on_stack.insert(id);
                    stack.push((id, true));
                    for &p in parents {
                        if !memo.contains_key(&p) && !on_stack.contains(&p) {
                            stack.push((p, false));
                        }
                    }
                    continue;
                }
                on_stack.remove(&id);
                let n = self.nodes.get(&id);
                let mut owned = n.is_some_and(|n| n.owner == Some(user));
                let mut nearest = self.roles.get(&(id, user)).map(|r| (0, *r));
                for p in parents {
                    // A parent still being walked is part of a loop; loops are refused when links
                    // are made, so it's skipped rather than followed.
                    let Some(&(p_owned, p_near)) = memo.get(p) else { continue };
                    owned |= p_owned;
                    if self.roles.contains_key(&(id, user)) {
                        continue;
                    }
                    if let Some((d, r)) = p_near {
                        let better = match nearest {
                            None => true,
                            Some((nd, nr)) => {
                                d + 1 < nd || (d + 1 == nd && r == Role::Contributor && nr == Role::Watcher)
                            }
                        };
                        if better {
                            nearest = Some((d + 1, r));
                        }
                    }
                }
                memo.insert(id, (owned, nearest));
            }
        }
        memo.into_iter()
            .map(|(id, (owned, nearest))| {
                let a = match (owned, nearest) {
                    (true, _) => Access::Own,
                    (_, Some((_, Role::Contributor))) => Access::Contribute,
                    (_, Some((_, Role::Watcher))) => Access::Watch,
                    _ => Access::None,
                };
                (id, a)
            })
            .collect()
    }

    /// Every private item, in one pass (see [`Graph::is_private`]).
    pub fn private_set(&self) -> HashSet<Id> {
        let mut memo: HashMap<Id, bool> = HashMap::with_capacity(self.nodes.len());
        for &start in self.nodes.keys() {
            let mut chain = Vec::new();
            let mut cur = Some(start);
            let mut result = false;
            while let Some(i) = cur {
                if let Some(&known) = memo.get(&i) {
                    result = known;
                    break;
                }
                if chain.contains(&i) {
                    break;
                }
                let Some(n) = self.nodes.get(&i) else { break };
                chain.push(i);
                if n.private {
                    result = true;
                    break;
                }
                cur = n.home;
            }
            // Everything walked shares the answer: below a private goal, or not private at all.
            for i in chain {
                memo.insert(i, result || self.nodes.get(&i).is_some_and(|n| n.private));
            }
        }
        memo.into_iter().filter(|(_, p)| *p).map(|(i, _)| i).collect()
    }

    /// Everything below `id` (through every link), for checking many goals for loops at once.
    pub fn below(&self, id: Id) -> HashSet<Id> {
        let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
        for (&c, n) in &self.nodes {
            for &p in &n.parents {
                children.entry(p).or_default().push(c);
            }
        }
        let mut out = HashSet::new();
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            for &c in children.get(&i).map(Vec::as_slice).unwrap_or(&[]) {
                if out.insert(c) {
                    stack.push(c);
                }
            }
        }
        out
    }

    /// Access from a precomputed role (from [`Graph::roles_for`]) and private set.
    pub fn access_with(&self, id: Id, role: Access, private: &HashSet<Id>) -> Access {
        let Some(n) = self.nodes.get(&id) else { return Access::None };
        if n.archived {
            return if role == Access::Own { Access::Own } else { Access::None };
        }
        if role > Access::None {
            return role;
        }
        if private.contains(&id) { Access::None } else { Access::Read }
    }

    pub fn access(&self, user: Id, id: Id) -> Access {
        let Some(n) = self.nodes.get(&id) else { return Access::None };
        let role = self.role(user, id);
        if n.archived {
            return if role == Access::Own { Access::Own } else { Access::None };
        }
        if role > Access::None {
            return role;
        }
        if self.is_private(id) { Access::None } else { Access::Read }
    }

    /// Everyone who owns `id`: its owner and the owner of every goal above it.
    pub fn owners(&self, id: Id) -> std::collections::BTreeSet<Id> {
        self.upward(id).filter_map(|i| self.nodes.get(&i).and_then(|n| n.owner)).collect()
    }

    /// Would changing `id`'s parents to `parents` (and its home to `home`) change who owns it or
    /// whether it's private? Contributors may rearrange links freely, but not in ways that hand
    /// the item to new owners, take it from existing ones, or change who can see it; those are
    /// an owner's decisions (decided 2026-11-16).
    pub fn relink_changes_control(&self, id: Id, parents: Vec<Id>, home: Option<Id>) -> bool {
        let mut after = self.clone();
        if let Some(n) = after.nodes.get_mut(&id) {
            n.parents = parents;
            n.home = home;
        }
        after.owners(id) != self.owners(id) || after.is_private(id) != self.is_private(id)
    }

    /// Would making `child` feed `goal` create a loop of goals containing themselves?
    pub fn would_contain_itself(&self, child: Id, goal: Id) -> bool {
        child == goal || self.upward(goal).any(|i| i == child)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(owner: Option<Id>, parents: &[Id]) -> Node {
        Node { owner, home: parents.first().copied(), parents: parents.to_vec(), private: false, archived: false }
    }

    // 1 (ada) ─┬─ 2 (goal, kofi) ── 4 (task)
    //          └─ 3 (goal, ada, private) ── 5 (task, also feeds 2; home is 3)
    //  6 (lin, top-level, public) ── 7 (task)
    fn graph() -> Graph {
        let (ada, kofi, lin) = (10, 11, 12);
        let mut g = Graph::default();
        g.nodes.insert(1, node(Some(ada), &[]));
        g.nodes.insert(2, node(Some(kofi), &[1]));
        g.nodes.insert(3, Node { private: true, ..node(Some(ada), &[1]) });
        g.nodes.insert(4, node(Some(kofi), &[2]));
        g.nodes.insert(5, node(Some(ada), &[3, 2]));
        g.nodes.insert(6, node(Some(lin), &[]));
        g.nodes.insert(7, node(Some(lin), &[6]));
        g
    }

    #[test]
    fn owners_above_own_everything_below() {
        let g = graph();
        assert_eq!(g.access(10, 4), Access::Own, "ada owns 1, so she owns 4 under 2");
        assert_eq!(g.access(11, 4), Access::Own);
        assert_eq!(g.access(11, 1), Access::Read, "owning a child gives nothing upward");
    }

    #[test]
    fn privacy_follows_the_home_chain() {
        let g = graph();
        assert!(g.is_private(3));
        assert!(g.is_private(5), "5's home is the private goal 3");
        assert!(!g.is_private(4));
        assert_eq!(g.access(12, 5), Access::None, "lin has no role, and 5 is private");
        assert_eq!(g.access(11, 5), Access::Own, "kofi owns 2, which 5 also feeds");
        assert_eq!(g.access(12, 4), Access::Read, "public items are readable by everyone");
    }

    #[test]
    fn nearest_membership_wins_and_flows_from_every_parent() {
        let mut g = graph();
        g.roles.insert((1, 12), Role::Watcher);
        g.roles.insert((2, 12), Role::Contributor);
        assert_eq!(g.access(12, 4), Access::Contribute, "Contributor on 2 is nearer than Watcher on 1");
        assert_eq!(g.access(12, 3), Access::Watch, "Watcher on 1 carries down to private 3");
        assert_eq!(g.access(12, 5), Access::Contribute, "5 also feeds 2");
        g.roles.insert((5, 12), Role::Watcher);
        assert_eq!(g.access(12, 5), Access::Watch, "a role on the item itself is nearest");
    }

    #[test]
    fn archived_items_are_for_owners_only() {
        let mut g = graph();
        g.nodes.get_mut(&7).unwrap().archived = true;
        g.roles.insert((6, 10), Role::Contributor);
        assert_eq!(g.access(12, 7), Access::Own);
        assert_eq!(g.access(10, 7), Access::None);
    }

    #[test]
    fn relinking_must_not_change_control() {
        let g = graph();
        // Linking 7 (lin's) under 2 (kofi's) would make kofi and ada owners of it.
        assert!(g.relink_changes_control(7, vec![6, 2], Some(6)));
        // Linking 4 (under 2, owned by kofi and ada) also under 1 adds no owners.
        assert!(!g.relink_changes_control(4, vec![2, 1], Some(2)));
        // Moving 5's home from private 3 to public 2 would make it public.
        assert!(g.relink_changes_control(5, vec![3, 2], Some(2)));
        // Removing 5's link to 2 takes kofi's ownership away.
        assert!(g.relink_changes_control(5, vec![3], Some(3)));
        assert_eq!(g.owners(5), [10, 11].into());
    }

    /// The one-pass versions agree with the item-by-item rules on many random graphs.
    #[test]
    fn one_pass_access_matches_the_rules() {
        let mut seed: u64 = 0x5eed;
        let mut rand = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for _ in 0..300 {
            let mut g = Graph::default();
            let size = 1 + rand(40) as Id;
            for id in 1..=size {
                // Parents only among earlier items, so there are no loops (as in fluxo).
                let mut parents: Vec<Id> = Vec::new();
                if id > 1 {
                    for _ in 0..rand(4) {
                        let p = 1 + rand(id as u64 - 1) as Id;
                        if !parents.contains(&p) {
                            parents.push(p);
                        }
                    }
                }
                let owner = if rand(5) == 0 { None } else { Some(10 + rand(4) as Id) };
                g.nodes.insert(
                    id,
                    Node {
                        owner,
                        home: parents.first().copied(),
                        parents,
                        private: rand(6) == 0,
                        archived: rand(10) == 0,
                    },
                );
            }
            for _ in 0..rand(12) {
                let role = if rand(2) == 0 { Role::Watcher } else { Role::Contributor };
                g.roles.insert((1 + rand(size as u64) as Id, 10 + rand(4) as Id), role);
            }
            let private = g.private_set();
            for user in 10..14 {
                let roles = g.roles_for(user);
                for &id in g.nodes.keys() {
                    assert_eq!(roles[&id], g.role(user, id), "role of {user} on {id} in {g:?}");
                    assert_eq!(g.access_with(id, roles[&id], &private), g.access(user, id));
                    assert_eq!(private.contains(&id), g.is_private(id));
                }
            }
            let x = 1 + rand(size as u64) as Id;
            let below = g.below(x);
            for &goal in g.nodes.keys() {
                assert_eq!(g.would_contain_itself(x, goal), goal == x || below.contains(&goal));
            }
        }
    }

    #[test]
    fn deep_trees_take_one_pass() {
        let mut g = Graph::default();
        g.nodes.insert(1, node(Some(10), &[]));
        for id in 2..=20_000 {
            g.nodes.insert(id, node(Some(11), &[id - 1]));
        }
        g.roles.insert((1, 12), Role::Watcher);
        let roles = g.roles_for(12);
        assert_eq!(roles[&20_000], Access::Watch);
        assert_eq!(g.roles_for(10)[&20_000], Access::Own);
        assert_eq!(g.private_set().len(), 0);
    }

    #[test]
    fn equally_near_roles_favour_contributor() {
        let mut g = graph();
        g.roles.insert((3, 12), Role::Watcher);
        g.roles.insert((2, 12), Role::Contributor);
        assert_eq!(g.access(12, 5), Access::Contribute, "5 sits directly under both 3 and 2");
        assert_eq!(g.roles_for(12)[&5], Access::Contribute);
    }

    #[test]
    fn goals_cannot_contain_themselves() {
        let g = graph();
        assert!(g.would_contain_itself(1, 4), "1 is above 4, so it can't also feed into 4");
        assert!(g.would_contain_itself(2, 2));
        assert!(!g.would_contain_itself(6, 2));
    }
}
