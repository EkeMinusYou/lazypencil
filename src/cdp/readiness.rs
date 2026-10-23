//! Optional main-document readiness, evaluated as data under the same budget.

use super::{Browser, Budget};
use crate::{Error, Result, config::runtime::PageSelectorState};
use std::time::Duration;

impl Browser {
    pub fn wait_for_selector(
        &mut self,
        session: &str,
        selector: &str,
        state: PageSelectorState,
        scroll: (f64, f64),
        budget: &Budget<'_>,
    ) -> Result<()> {
        if selector.is_empty() {
            return Ok(());
        }
        let selector = serde_json::to_string(selector).expect("A selector string is serializable");
        let attached = state == PageSelectorState::Attached;
        let expression = format!(
            "(()=>{{window.scrollTo({{left:{},top:{},behavior:'instant'}});\
            let element;try{{element=document.querySelector({selector});}}catch{{return 'invalid';}}\
            if(!element)return 'pending';if({attached})return 'ready';\
            const visible=element.checkVisibility({{opacityProperty:true,visibilityProperty:true,contentVisibilityAuto:true}})&&\
            [...element.getClientRects()].some(rect=>rect.width>0&&rect.height>0);\
            return visible?'ready':'pending';}})()",
            scroll.0, scroll.1
        );
        loop {
            match self.evaluate(session, &expression, budget)?.as_str() {
                Some("ready") => return Ok(()),
                Some("invalid") => {
                    return Err(Error::new(
                        "PAGE_LOAD_FAILED",
                        "Invalid page readiness selector",
                    ));
                }
                Some("pending") => (),
                _ => {
                    return Err(Error::new(
                        "PAGE_LOAD_FAILED",
                        "Invalid page readiness result",
                    ));
                }
            }
            std::thread::sleep(Duration::from_millis(20).min(budget.remaining()?));
        }
    }
}
