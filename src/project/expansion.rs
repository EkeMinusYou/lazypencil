use super::*;

/// Bound allocation work, not process RSS. Count values before cloning them,
/// including nested strings and map entries, across every root in an operation.
pub(crate) const MAX_EXPANSION_BYTES: usize = 128 * 1024 * 1024;

pub(crate) struct Expansion {
    pub nodes: usize,
    remaining: usize,
}

impl Default for Expansion {
    fn default() -> Self {
        Self {
            nodes: 0,
            remaining: MAX_EXPANSION_BYTES,
        }
    }
}

impl Expansion {
    #[cfg(feature = "rendering")]
    pub fn scene(roots: &[ResolvedNode]) -> Result<Self> {
        let mut budget = Self::default();
        for root in roots {
            budget.node(root)?;
        }
        Ok(budget)
    }

    #[cfg(feature = "rendering")]
    fn node(&mut self, node: &ResolvedNode) -> Result<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "Expanded scene exceeds its node limit",
            ));
        }
        self.reserve(std::mem::size_of::<ResolvedNode>())?;
        self.object(&node.properties)?;
        for part in &node.address.instance_path {
            self.key(part)?;
        }
        for (key, source) in &node.property_sources {
            self.key(key)?;
            self.reserve(source.as_os_str().len())?;
        }
        for child in &node.children {
            self.node(child)?;
        }
        Ok(())
    }

    pub fn reserve(&mut self, bytes: usize) -> Result<()> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(|| {
            Error::new(
                "RESOURCE_LIMIT",
                "Expanded document exceeds its allocation budget",
            )
            .details(json!({"limit_bytes": MAX_EXPANSION_BYTES}))
        })?;
        Ok(())
    }

    pub fn value(&mut self, value: &Value) -> Result<()> {
        self.reserve(std::mem::size_of::<Value>())?;
        match value {
            Value::String(text) => self.reserve(text.len()),
            Value::Array(values) => {
                for value in values {
                    self.value(value)?;
                }
                Ok(())
            }
            Value::Object(values) => self.object(values),
            _ => Ok(()),
        }
    }

    pub fn object(&mut self, values: &Map<String, Value>) -> Result<()> {
        for (key, value) in values {
            self.key(key)?;
            self.value(value)?;
        }
        Ok(())
    }

    pub fn key(&mut self, key: &str) -> Result<()> {
        // Account conservatively for owned keys and tree entry bookkeeping.
        self.reserve(key.len())?;
        self.reserve(96)
    }

    pub fn clone_value(&mut self, value: &Value) -> Result<Value> {
        self.value(value)?;
        Ok(value.clone())
    }
}

#[cfg(test)]
mod tests;
