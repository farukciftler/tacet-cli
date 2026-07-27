//! ToolCatalog — the set of tools available at run time.
//!
//! Single source of truth: the prompt text, the grammar and the call routing
//! all derive from this list. No second "tool names" list is kept; if one were,
//! it would drift sooner or later and the model would call a tool that does not
//! exist.

use crate::tool::Tool;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ToolCatalog {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        self.tools.push(tool);
        self
    }

    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    /// Looks up by name. If the model sends an invented name this returns `None`
    /// and the call site turns it into the fixed error text.
    ///
    /// CASE IS IGNORED — but an EXACT match is tried first.
    ///
    /// WHY: in a tool-selection measurement the model picked the right tool and
    /// capitalized its name — `Time(...)` for "what is today's date?", and
    /// `Web_ara(...)` in a web case. The selection was correct, the call was
    /// still rejected at gate 1, and the user got no answer. This is not a
    /// SECURITY gate, it was a SPELLING gate: the rejection protected nothing,
    /// because the name is already in the catalog. In a small model the
    /// capitalize-at-sentence-start reflex is unavoidable; having the catalog
    /// forgive it is both cheaper and more reliable than writing one more rule
    /// into the prompt.
    ///
    /// LIMIT: only letter case is forgiven. An invented name still returns
    /// `None` — the gate's real job stands. Tool names are ASCII snake_case and
    /// differ from each other by MORE than case (see the catalog test), so this
    /// relaxation cannot confuse two tools.
    pub fn find(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if let Some(t) = self.tools.iter().find(|t| t.name() == name) {
            return Some(Arc::clone(t));
        }
        self.tools
            .iter()
            .find(|t| t.name().eq_ignore_ascii_case(name))
            .cloned()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    /// Tools that read personal data — whoever sets the approval policy reads this.
    pub fn tainting_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|t| t.taints_session())
            .map(|t| t.name())
            .collect()
    }
}

impl FromIterator<Arc<dyn Tool>> for ToolCatalog {
    fn from_iter<T: IntoIterator<Item = Arc<dyn Tool>>>(iter: T) -> Self {
        Self {
            tools: iter.into_iter().collect(),
        }
    }
}
