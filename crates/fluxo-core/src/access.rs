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
    /// otherwise the nearest membership. `Access::None` if they have no role.
    pub fn role(&self, user: Id, id: Id) -> Access {
        let mut nearest = None;
        for i in self.upward(id) {
            if self.nodes.get(&i).is_some_and(|n| n.owner == Some(user)) {
                return Access::Own;
            }
            if nearest.is_none() {
                nearest = self.roles.get(&(i, user)).copied();
            }
        }
        match nearest {
            Some(Role::Contributor) => Access::Contribute,
            Some(Role::Watcher) => Access::Watch,
            None => Access::None,
        }
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

    #[test]
    fn goals_cannot_contain_themselves() {
        let g = graph();
        assert!(g.would_contain_itself(1, 4), "1 is above 4, so it can't also feed into 4");
        assert!(g.would_contain_itself(2, 2));
        assert!(!g.would_contain_itself(6, 2));
    }
}
