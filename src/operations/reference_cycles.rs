//! Iterative cycle detection over stored child and local-reference edges.
use super::*;

enum Visit<'a> {
    Enter(&'a str),
    Leave(&'a str),
}

pub(super) fn check(document: &Document, check: &dyn Fn() -> Result<()>) -> Result<()> {
    let mut active = HashSet::new();
    let mut completed = HashSet::new();
    let mut pending = Vec::new();
    for id in document.locations().keys() {
        pending.push(Visit::Enter(id));
        while let Some(visit) = pending.pop() {
            check()?;
            match visit {
                Visit::Enter(id) => {
                    if completed.contains(id) {
                        continue;
                    }
                    if !active.insert(id) {
                        return Err(Error::new(
                            "REFERENCE_CYCLE",
                            format!("Component cycle at {id}"),
                        ));
                    }
                    let node = document.node(id)?;
                    pending.push(Visit::Leave(id));
                    if let Some(children) = node.get("children").and_then(Value::as_array) {
                        for child in children.iter().rev() {
                            if let Some(id) = child["id"].as_str() {
                                pending.push(Visit::Enter(id));
                            }
                        }
                    }
                    if node["type"] == "ref"
                        && let Some(target) = node.get("ref").and_then(Value::as_str)
                        && document.node(target).is_ok()
                    {
                        pending.push(Visit::Enter(target));
                    }
                }
                Visit::Leave(id) => {
                    active.remove(id);
                    completed.insert(id);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn graph(length: usize, branches: bool, cycle: bool) -> Document {
        let mut nodes = vec![json!({"id":"000-first","type":"ref","ref":"c0"})];
        for index in 0..length {
            let target = format!("c{}", index + 1);
            nodes.push(if branches {
                json!({"id":format!("c{index}"),"type":"frame","reusable":true,
                    "children":[{"id":format!("a{index}"),"type":"ref","ref":target},
                    {"id":format!("b{index}"),"type":"ref","ref":target}]})
            } else {
                json!({"id":format!("c{index}"),"type":"ref","reusable":true,"ref":target})
            });
        }
        nodes.push(if cycle {
            json!({"id":format!("c{length}"),"type":"ref","reusable":true,"ref":"c0"})
        } else {
            json!({"id":format!("c{length}"),"type":"frame","reusable":true})
        });
        Document::parse(json!({"version":"2.22","children":nodes}).to_string()).unwrap()
    }

    #[test]
    fn shared_reference_graphs_have_linear_work_and_cycles_still_fail() {
        let document = graph(40, true, false);
        let visits = Cell::new(0);
        check(&document, &|| {
            visits.set(visits.get() + 1);
            assert!(
                visits.get() < 1000,
                "Shared references must not multiply work"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            check(&graph(40, true, true), &|| Ok(())).unwrap_err().code,
            "REFERENCE_CYCLE"
        );
    }

    #[test]
    fn deep_references_use_heap_storage_and_cancel_during_traversal() {
        let document = graph(10_000, false, false);
        check(&document, &|| Ok(())).unwrap();
        let visits = Cell::new(0);
        let cancel = tokio_util::sync::CancellationToken::new();
        let error = check(&document, &|| {
            visits.set(visits.get() + 1);
            if visits.get() == 100 {
                cancel.cancel();
            }
            crate::cancel::check(Some(&cancel))
        })
        .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
    }

    #[test]
    fn child_edges_cannot_hide_an_ancestor_reference_cycle() {
        let document = Document::parse(
            json!({"version":"2.22","children":[
            {"id":"parent","type":"frame","reusable":true,"children":[
                {"id":"child","type":"ref","ref":"parent"}]}]})
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            check(&document, &|| Ok(())).unwrap_err().code,
            "REFERENCE_CYCLE"
        );
    }
}
