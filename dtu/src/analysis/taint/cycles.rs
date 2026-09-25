use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::{delete, insert_into, insert_or_ignore_into, SqliteConnection};

use super::engine::IdFactories;
use super::models::{InsertEdge, InsertNode, Node, NodeId};
use super::schema::{edges, graph_metadata, nodes, reachable_nodes};
use crate::db::graph::models::MethodId;
use crate::db::{self, query, query_exec};

/// Unrolling is exponential in the size of a cycle, so past this many copies it is cut instead.
/// Observed cycles are 2-4 nodes and need at most a handful.
const MAX_COPIES: usize = 4096;

// We use Vec's here that are keyed by NodeId instead of HashMaps because there really shouldn't be
// many gaps in the NodeId values.

struct Adjacency {
    sources: Vec<NodeId>,
    offsets: Vec<usize>,
    targets: Vec<(NodeId, MethodId)>,
}

impl Adjacency {
    fn load(conn: &mut SqliteConnection) -> db::Result<Self> {
        let rows = query!(edges::table
            .select((edges::src, edges::dst, edges::location))
            .order_by((edges::src, edges::dst, edges::location)))
        .load::<(NodeId, NodeId, MethodId)>(conn)?;

        // We'll build our Vec to be sized to max(NodeId) + 1

        let max_node_id = rows
            .iter()
            .map(|(src, dst, _)| slot(*src).max(slot(*dst)) + 1)
            .max()
            .unwrap_or(0);

        // Since we got the rows sorted, we can get the successors by a range. Since this took my
        // small monkey brain some time here is a toy example. Say only 3 edges are in there:
        //
        // 1 -> 2
        // 1 -> 3
        // 3 -> 1
        //
        // These are NodeIds [1, 2, 3] (note that we never have NodeId 0). These NodeIds will be
        // used as indices into offsets and targets: offsets defines sliced views of targets. So in
        // this toy case offsets is:
        //
        // [0, 0, 2, 2, 3]
        //
        // and targets is (location dropped):
        //
        // [ 2, 3, 1 ]
        //
        // And we can see that slices based on NodeId do indeed return the correct targets:
        //
        // Node 1 -> &targets[offsets[1]..offsets[2]] -> &targets[0..2] -> [2, 3]
        // Node 2 -> &targets[offsets[2]..offsets[3]] -> &targets[2..2] -> []
        // Node 3 -> &targets[offsets[3]..offsets[4]] -> &targets[2..3] -> [1]
        //
        // Once again, this is only valid because of the `ORDER BY` above making the results sorted.

        let mut offsets = vec![0usize; max_node_id + 1];

        // So to actually build offsets we just do offsets[node + 1] += 1 while looping through:
        //
        // Node N -> offsets[N + 1] += 1
        //
        // This essentially defines the length of the slice.

        for (src, _, _) in &rows {
            offsets[slot(*src) + 1] += 1;
        }

        // Then we need to do a running sum here to make the offsets absolute instead of relative.
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }

        let mut sources = rows.iter().map(|(src, _, _)| *src).collect::<Vec<_>>();
        sources.dedup();

        let targets = rows
            .into_iter()
            .map(|(_, dst, location)| (dst, location))
            .collect();

        Ok(Self {
            sources,
            offsets,
            targets,
        })
    }

    /// The number of slots in a Vec indexed by NodeId
    ///
    /// This is not the number of nodes, there may be gaps and 0 is never used.
    fn num_slots(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn successors(&self, node: NodeId) -> &[(NodeId, MethodId)] {
        let idx = slot(node);
        match (self.offsets.get(idx), self.offsets.get(idx + 1)) {
            (Some(&start), Some(&end)) => &self.targets[start..end],
            _ => &[],
        }
    }
}

fn slot(node: NodeId) -> usize {
    usize::try_from(node.raw()).expect("invalid NodeId")
}

/// Rewrite every cycle in the edges table so the graph is acyclic
///
/// A lot of cycles are removed from the graph during creation over in the engine, but some will be
/// created due to the work caching we do. Cleaning them up is relatively cheap so we do it here.
pub(super) fn break_cycles(conn: &mut SqliteConnection, factories: &IdFactories) -> db::Result<()> {
    let adj = Adjacency::load(conn)?;
    let components = Tarjan::cyclic_components(&adj);
    if components.is_empty() {
        // Yay no cycles
        return Ok(());
    }

    let mut copies = Vec::new();
    let mut edges = Vec::new();

    log::info!("breaking {} cycles in the taint graph", components.len());
    for component in components {
        break_cycle(conn, factories, &adj, &component, &mut copies, &mut edges)?;

        copies.clear();
        edges.clear();
    }
    Ok(())
}

/// Break up a single SCC
///
/// We break cycles by essentially walking through the SCC and recreating the nodes with edges
/// strictly inside of it while removing any edges that reference the current path (a cycle). This
/// is bound by a maximum copy size, and, if that size is reached, we instead just cut out the edge.
/// Edges that enter or leave the SCC have no changes.
fn break_cycle(
    conn: &mut SqliteConnection,
    factories: &IdFactories,
    adj: &Adjacency,
    component: &[NodeId],
    copies: &mut Vec<(NodeId, NodeId)>,
    edges: &mut Vec<InsertEdge>,
) -> db::Result<()> {
    // A set of every node in the cycle
    let members = component.iter().copied().collect::<HashSet<_>>();

    let checkpoint = factories.nodes.checkpoint_dangerous();

    let mut unroll = Unroll {
        adj,
        members: &members,
        copies,
        edges,
    };

    let mut work = Vec::new();
    let unrolled = component.iter().all(|&root| {
        let expanded = unroll.expand(root, &mut work, factories);
        work.clear();
        expanded
    });

    let copies = if !unrolled {
        log::warn!(
            "cycle through {} nodes is too large to unroll, cutting it instead",
            component.len()
        );

        // Revert the minted nodes so we don't leave large gaps
        factories.nodes.revert_dangerous(checkpoint);

        unroll.edges.clear();
        forward_edges(adj, &members, unroll.edges);
        None
    } else {
        let originals = query!(nodes::table.filter(nodes::id.eq_any(component)))
            .load::<Node>(conn)?
            .into_iter()
            .map(|it| (it.id, it))
            .collect::<HashMap<_, _>>();

        Some((unroll.copies, originals))
    };

    let new_nodes = copies.map(|(copies, originals)| {
        copies
            .iter()
            .filter_map(|(id, original)| {
                let node = originals.get(original)?;
                Some(InsertNode {
                    id: *id,
                    kind: node.kind,
                    sink_id: node.sink_id,
                    graph_id: node.graph_id,
                    regs: node.regs.clone(),
                })
            })
            .collect::<Vec<_>>()
    });

    clear_reachable(conn, component)?;
    query_exec!(
        delete(
            edges::table
                .filter(edges::src.eq_any(component))
                .filter(edges::dst.eq_any(component))
        ),
        conn
    )?;
    if let Some(nodes) = new_nodes {
        query_exec!(insert_into(nodes::table).values(&nodes), conn)?;
    }
    query_exec!(
        insert_or_ignore_into(edges::table).values(unroll.edges as &Vec<InsertEdge>),
        conn
    )?;
    Ok(())
}

fn clear_reachable(conn: &mut SqliteConnection, component: &[NodeId]) -> db::Result<()> {
    let graphs = query!(reachable_nodes::table
        .select(reachable_nodes::graph)
        .filter(reachable_nodes::node.eq_any(component))
        .distinct())
    .load::<i32>(conn)?;

    if graphs.is_empty() {
        return Ok(());
    }

    query_exec!(
        delete(graph_metadata::table.filter(graph_metadata::graph.eq_any(&graphs))),
        conn
    )?;
    query_exec!(
        delete(reachable_nodes::table.filter(reachable_nodes::graph.eq_any(&graphs))),
        conn
    )?;
    Ok(())
}

/// The internal edges that go from a lower to a higher id, which can't form a cycle
fn forward_edges(adj: &Adjacency, members: &HashSet<NodeId>, edges: &mut Vec<InsertEdge>) {
    edges.extend(members.iter().flat_map(|&src| {
        adj.successors(src)
            .iter()
            .filter(move |(dst, _)| members.contains(dst) && src < *dst)
            .map(move |&(dst, location)| InsertEdge { src, dst, location })
    }))
}

struct Unroll<'a> {
    adj: &'a Adjacency,
    members: &'a HashSet<NodeId>,
    /// (copy, original)
    copies: &'a mut Vec<(NodeId, NodeId)>,
    edges: &'a mut Vec<InsertEdge>,
}

struct Frame<'a> {
    node: NodeId,
    original: NodeId,
    successors: &'a [(NodeId, MethodId)],
    next: usize,
}

impl<'a> Frame<'a> {
    fn new(adj: &'a Adjacency, node: NodeId, original: NodeId) -> Self {
        let successors = adj.successors(original);
        Self {
            successors,
            node,
            original,
            next: 0,
        }
    }

    fn next_successor(&mut self) -> Option<&'a (NodeId, MethodId)> {
        let succ = self.successors.get(self.next)?;
        self.next += 1;
        Some(succ)
    }

    fn is_copy(&self) -> bool {
        self.node != self.original
    }
}

impl<'a> Unroll<'a> {
    /// Expand will essentially recreate parts of the graph with new NodeIds but with the cycle
    /// removed. This maintains as much information as we can while also eliminating the cycle. This
    /// is kinda an analogue to what we do over in engine with the call stack
    fn expand(&mut self, node: NodeId, work: &mut Vec<Frame<'a>>, factories: &IdFactories) -> bool {
        work.push(Frame::new(&self.adj, node, node));

        while let Some(frame) = work.last_mut() {
            let Some(&(dst, location)) = frame.next_successor() else {
                work.pop();
                continue;
            };

            let node = frame.node;

            // If the dest node isn't in the cycle, we don't need to do anything unless we're making
            // a copy
            if !self.members.contains(&dst) {
                // If it is a copy, we need to maintain the correct edges since that new copy won't
                // have any
                if frame.is_copy() {
                    self.edges.push(InsertEdge {
                        src: node,
                        dst,
                        location,
                    });
                }

                // Note that we still keep it in the work queue: we'll move on to the next dst node
                continue;
            }

            // Check to see if it's on the path, this is kinda the whole point :)
            if work.iter().any(|it| it.original == dst) {
                continue;
            }

            // Reaching here means that this node stays inside the SCC and we have to follow it

            // Just a safeguard here to stop some potential exponential explosion. I haven't seen it
            // in real databases, but worth while.
            if self.copies.len() >= MAX_COPIES {
                return false;
            }

            // We need to make the copy now: mint a new NodeId and create new edges for it

            let copy = factories.new_node_id();
            self.copies.push((copy, dst));

            // Recreate the edge
            self.edges.push(InsertEdge {
                src: node,
                dst: copy,
                location,
            });

            work.push(Frame::new(&self.adj, copy, dst));
        }
        true
    }
}

// Thanks Wikipedia, but instead of recursion we use a work queue

struct Tarjan {
    next_index: usize,
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    stack: Vec<NodeId>,
    on_stack: Vec<bool>,
    cyclic: Vec<Vec<NodeId>>,
}

impl Tarjan {
    fn cyclic_components(adj: &Adjacency) -> Vec<Vec<NodeId>> {
        let slots = adj.num_slots();
        let mut tarjan = Self {
            next_index: 0,
            index: vec![None; slots],
            low: vec![0; slots],
            stack: Vec::new(),
            on_stack: vec![false; slots],
            cyclic: Vec::new(),
        };

        // Chose a kinda arbitrary number
        let mut work = Vec::with_capacity(16);

        for &root in &adj.sources {
            if tarjan.index[slot(root)].is_none() {
                tarjan.visit(adj, root, &mut work);
            }
        }
        tarjan.cyclic
    }

    fn visit(&mut self, adj: &Adjacency, root: NodeId, work: &mut Vec<(NodeId, usize)>) {
        self.enter(root);
        work.push((root, 0));

        while let Some(top) = work.last_mut() {
            let (node, pos) = *top;
            top.1 += 1;

            match adj.successors(node).get(pos) {
                Some(&(dst, _)) => self.step(node, dst, work),
                None => self.leave(adj, node, work),
            }
        }
    }

    fn enter(&mut self, node: NodeId) {
        let idx = slot(node);
        self.index[idx] = Some(self.next_index);
        self.low[idx] = self.next_index;
        self.next_index += 1;
        self.stack.push(node);
        self.on_stack[idx] = true;
    }

    fn step(&mut self, node: NodeId, dst: NodeId, work: &mut Vec<(NodeId, usize)>) {
        let dst_idx = slot(dst);
        match self.index[dst_idx] {
            None => {
                self.enter(dst);
                work.push((dst, 0));
            }
            Some(index) if self.on_stack[dst_idx] => self.lower(node, index),
            Some(_) => {}
        }
    }

    fn leave(&mut self, adj: &Adjacency, node: NodeId, work: &mut Vec<(NodeId, usize)>) {
        work.pop();
        let idx = slot(node);
        let low = self.low[idx];
        if let Some(&(parent, _)) = work.last() {
            self.lower(parent, low);
        }
        if Some(low) == self.index[idx] {
            self.pop_component(adj, node);
        }
    }

    fn lower(&mut self, node: NodeId, value: usize) {
        let low = &mut self.low[slot(node)];
        *low = (*low).min(value);
    }

    fn pop_component(&mut self, adj: &Adjacency, root: NodeId) {
        let mut component = Vec::new();
        while let Some(node) = self.stack.pop() {
            self.on_stack[slot(node)] = false;
            component.push(node);
            if node == root {
                break;
            }
        }

        // If the node recurses into itself (which I don't think can happen with the way we handle
        // things over in the engine, but we might as well keep this defensively) or the SCC has
        // more than 1 node we have a cycle.

        let self_loop = adj.successors(root).iter().any(|(dst, _)| *dst == root);
        if component.len() > 1 || self_loop {
            self.cyclic.push(component);
        }
    }
}
