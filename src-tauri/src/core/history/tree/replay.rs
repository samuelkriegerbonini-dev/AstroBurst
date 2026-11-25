use crate::core::history::doc::{node_number, HistoryDoc, Node};
use crate::core::history::error::HistoryError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPlan {
    pub from: Option<String>,
    pub steps: Vec<String>,
}

fn newest_descent<'a>(doc: &'a HistoryDoc, from: Option<&str>) -> Vec<&'a Node> {
    let mut tail: Vec<&Node> = Vec::new();
    let mut cursor = from.map(str::to_string);
    while let Some(child) = doc.nodes.iter().filter(|n| n.parent == cursor).max_by_key(|n| node_number(&n.id)) {
        if tail.len() == doc.nodes.len() {
            break;
        }
        cursor = Some(child.id.clone());
        tail.push(child);
    }
    tail
}

pub fn downstream_of(doc: &HistoryDoc, id: &str) -> Vec<Node> {
    let line = doc.head.as_deref().map(|head| doc.ancestry(head)).unwrap_or_default();
    let tail: Vec<&Node> = match line.iter().position(|n| n.id == id) {
        Some(at) => line[at + 1..].iter().copied().chain(newest_descent(doc, doc.head.as_deref())).collect(),
        None => newest_descent(doc, Some(id)),
    };
    tail.into_iter().cloned().collect()
}

pub fn replay_plan(doc: &HistoryDoc, target: &str) -> Result<ReplayPlan, HistoryError> {
    let path = doc.ancestry(target);
    if path.is_empty() {
        return Err(HistoryError::UnknownNode(target.to_string()));
    }
    let start = path.iter().rposition(|node| node.present);
    let from = start.map(|i| path[i].id.clone());
    let steps = path[start.map_or(0, |i| i + 1)..].iter().map(|node| node.id.clone()).collect();
    Ok(ReplayPlan { from, steps })
}
