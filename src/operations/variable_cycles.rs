//! Iterative, cancellable DFS shared across every local variable definition.
use super::*;

enum Visit<'a> {
    Enter(&'a str),
    Value(&'a Value),
    Leave(&'a str),
}

pub(super) fn check(variables: &Map<String, Value>, check: &dyn Fn() -> Result<()>) -> Result<()> {
    let mut active = HashSet::new();
    let mut completed = HashSet::new();
    let mut pending = Vec::new();
    for name in variables.keys() {
        pending.push(Visit::Enter(name));
        while let Some(visit) = pending.pop() {
            check()?;
            match visit {
                Visit::Enter(name) => {
                    if completed.contains(name) {
                        continue;
                    }
                    if !active.insert(name) {
                        return Err(Error::new(
                            "REFERENCE_CYCLE",
                            format!("Variable cycle at {name}"),
                        ));
                    }
                    pending.push(Visit::Leave(name));
                    pending.push(Visit::Value(&variables[name]["value"]));
                }
                Visit::Leave(name) => {
                    active.remove(name);
                    completed.insert(name);
                }
                Visit::Value(Value::String(value)) => {
                    if let Some((name, _)) = value
                        .strip_prefix('$')
                        .and_then(|name| variables.get_key_value(name))
                    {
                        pending.push(Visit::Enter(name));
                    }
                }
                Visit::Value(Value::Array(values)) => {
                    pending.extend(values.iter().rev().map(Visit::Value));
                }
                Visit::Value(Value::Object(values)) => {
                    pending.extend(
                        values
                            .iter()
                            .rev()
                            .filter(|(key, _)| !literal_variable_property(key))
                            .map(|(_, value)| Visit::Value(value)),
                    );
                }
                Visit::Value(_) => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn aliases(length: usize, shared: bool) -> Map<String, Value> {
        let mut variables = Map::new();
        for index in 0..length {
            let value = if shared {
                json!([{"value":format!("$v{}", index + 1)}, {"value":format!("$v{}", index + 1)}])
            } else {
                json!(format!("$v{}", index + 1))
            };
            variables.insert(format!("v{index}"), json!({"type":"number","value":value}));
        }
        variables.insert(format!("v{length}"), json!({"type":"number","value":1}));
        variables
    }

    #[test]
    fn shared_alias_branches_have_bounded_work_and_still_reject_cycles() {
        let mut variables = aliases(40, true);
        let visits = Cell::new(0);
        check(&variables, &|| {
            visits.set(visits.get() + 1);
            assert!(
                visits.get() < 1000,
                "Shared branches must not multiply work"
            );
            Ok(())
        })
        .unwrap();
        variables["v40"]["value"] = json!("$v0");
        assert_eq!(
            check(&variables, &|| Ok(())).unwrap_err().code,
            "REFERENCE_CYCLE"
        );
    }

    #[test]
    fn long_alias_chains_use_heap_storage_and_remain_cancellable_during_traversal() {
        let variables = aliases(10_000, false);
        check(&variables, &|| Ok(())).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let visits = Cell::new(0);
        let error = check(&variables, &|| {
            visits.set(visits.get() + 1);
            if visits.get() == 100 {
                cancel.cancel();
            }
            crate::cancel::check(Some(&cancel))
        })
        .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
    }
}
