use serde_json::{json, Value};

pub struct ChainBuilder {
    chain: Vec<Value>,
    pending_thinking: String,
}

impl ChainBuilder {
    pub fn new() -> Self {
        Self { chain: Vec::new(), pending_thinking: String::new() }
    }

    pub fn text_delta(&mut self, text: &str) {
        self.pending_thinking.push_str(text);
    }

    pub fn thinking(&mut self, text: &str) {
        self.flush_pending();
        if !text.is_empty() {
            self.chain.push(json!({ "type": "thinking", "text": text }));
        }
    }

    pub fn tool_call(&mut self, name: &str, input: &Value, description: Option<&str>, hostname: Option<&str>) {
        self.flush_pending();
        let mut entry = json!({
            "type": "tool_call",
            "name": name,
            "input": input,
            "status": "done",
            "preview": Value::Null,
        });
        if let Some(d) = description {
            entry["description"] = json!(d);
        }
        if let Some(h) = hostname {
            entry["hostname"] = json!(h);
        }
        self.chain.push(entry);
    }

    pub fn confirm_action(&mut self, id: &str, name: &str, input: &Value, description: &str) {
        self.flush_pending();
        self.chain.push(json!({
            "type": "confirm_action",
            "id": id,
            "name": name,
            "input": input,
            "description": description,
            "status": "pending",
            "steps": [],
            "result_text": "",
        }));
    }

    pub fn tool_result(&mut self, name: &str, preview: &str) {
        if let Some(entry) = self.chain.iter_mut()
            .find(|e| e["type"] == "tool_call" && e["name"] == name && e["preview"].is_null())
        {
            entry["preview"] = json!(preview);
        }
    }

    fn flush_pending(&mut self) {
        if !self.pending_thinking.is_empty() {
            let text = std::mem::take(&mut self.pending_thinking);
            self.chain.push(json!({ "type": "thinking", "text": text }));
        }
    }

    pub fn finish(self) -> Vec<Value> {
        self.chain
    }
}

impl Default for ChainBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builder_has_empty_chain() {
        let b = ChainBuilder::new();
        assert_eq!(b.finish(), Vec::<Value>::new());
    }

    #[test]
    fn text_delta_alone_produces_empty_chain() {
        let mut b = ChainBuilder::new();
        b.text_delta("hello");
        assert_eq!(b.finish(), Vec::<Value>::new());
    }

    #[test]
    fn thinking_pushes_immediately() {
        let mut b = ChainBuilder::new();
        b.thinking("reasoning");
        assert_eq!(b.finish(), vec![json!({"type": "thinking", "text": "reasoning"})]);
    }

    #[test]
    fn empty_thinking_text_is_not_pushed() {
        let mut b = ChainBuilder::new();
        b.thinking("");
        assert_eq!(b.finish(), Vec::<Value>::new());
    }

    #[test]
    fn text_delta_then_tool_call_promotes_to_thinking_before_tool_call() {
        let mut b = ChainBuilder::new();
        b.text_delta("let me check");
        b.tool_call("search", &json!({}), None, None);
        assert_eq!(b.finish(), vec![
            json!({"type": "thinking", "text": "let me check"}),
            json!({"type": "tool_call", "name": "search", "input": {}, "status": "done", "preview": null}),
        ]);
    }

    #[test]
    fn tool_call_without_preamble_has_no_thinking_before_it() {
        let mut b = ChainBuilder::new();
        b.tool_call("search", &json!({}), None, None);
        assert_eq!(b.finish(), vec![
            json!({"type": "tool_call", "name": "search", "input": {}, "status": "done", "preview": null}),
        ]);
    }

    #[test]
    fn text_delta_accumulates_across_multiple_deltas_before_promotion() {
        let mut b = ChainBuilder::new();
        b.text_delta("Hello ");
        b.text_delta("world");
        b.tool_call("search", &json!({}), None, None);
        assert_eq!(b.finish()[0], json!({"type": "thinking", "text": "Hello world"}));
    }

    #[test]
    fn tool_result_fills_matching_pending_preview() {
        let mut b = ChainBuilder::new();
        b.tool_call("search", &json!({"q": "x"}), None, None);
        b.tool_result("search", "found 3 results");
        assert_eq!(b.finish(), vec![
            json!({"type": "tool_call", "name": "search", "input": {"q": "x"}, "status": "done", "preview": "found 3 results"}),
        ]);
    }

    #[test]
    fn tool_result_for_unknown_name_is_noop() {
        let mut b = ChainBuilder::new();
        b.tool_call("search", &json!({}), None, None);
        b.tool_result("other_tool", "irrelevant");
        assert_eq!(b.finish()[0]["preview"], Value::Null);
    }

    #[test]
    fn multiple_tool_calls_same_name_fill_earliest_pending_preview_first() {
        let mut b = ChainBuilder::new();
        b.tool_call("fn", &json!({}), None, None);
        b.tool_call("fn", &json!({}), None, None);
        b.tool_result("fn", "result");
        let chain = b.finish();
        assert_eq!(chain[0]["preview"], json!("result"));
        assert_eq!(chain[1]["preview"], Value::Null);
    }

    #[test]
    fn chain_preserves_interleaved_order_of_thinking_and_tool_calls() {
        let mut b = ChainBuilder::new();
        b.thinking("a");
        b.tool_call("t1", &json!({}), None, None);
        b.tool_result("t1", "r1");
        b.text_delta("b");
        b.tool_call("t2", &json!({}), None, None);
        b.tool_result("t2", "r2");

        let chain = b.finish();
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0], json!({"type": "thinking", "text": "a"}));
        assert_eq!(chain[1], json!({"type": "tool_call", "name": "t1", "input": {}, "status": "done", "preview": "r1"}));
        assert_eq!(chain[2], json!({"type": "thinking", "text": "b"}));
        assert_eq!(chain[3], json!({"type": "tool_call", "name": "t2", "input": {}, "status": "done", "preview": "r2"}));
    }

    #[test]
    fn tool_call_includes_description_and_hostname_when_provided() {
        let mut b = ChainBuilder::new();
        b.tool_call("run_command", &json!({"agent_id": "a1"}), Some("Running ls"), Some("build-box"));
        let chain = b.finish();
        assert_eq!(chain[0]["description"], json!("Running ls"));
        assert_eq!(chain[0]["hostname"], json!("build-box"));
    }

    #[test]
    fn tool_call_omits_description_and_hostname_when_absent() {
        let mut b = ChainBuilder::new();
        b.tool_call("search", &json!({}), None, None);
        let chain = b.finish();
        assert!(chain[0].get("description").is_none());
        assert!(chain[0].get("hostname").is_none());
    }

    #[test]
    fn confirm_action_pushes_pending_entry_in_place() {
        let mut b = ChainBuilder::new();
        b.tool_call("list_agents", &json!({}), None, None);
        b.tool_result("list_agents", "[]");
        b.confirm_action("tc1", "create_lxd_agent", &json!({"name": "x"}), "Create agent x");
        let chain = b.finish();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[1], json!({
            "type": "confirm_action", "id": "tc1", "name": "create_lxd_agent",
            "input": {"name": "x"}, "description": "Create agent x",
            "status": "pending", "steps": [], "result_text": "",
        }));
    }

    #[test]
    fn confirm_action_promotes_pending_text_to_thinking_before_it() {
        let mut b = ChainBuilder::new();
        b.text_delta("Let me create that agent");
        b.confirm_action("tc1", "create_lxd_agent", &json!({}), "desc");
        let chain = b.finish();
        assert_eq!(chain[0], json!({"type": "thinking", "text": "Let me create that agent"}));
        assert_eq!(chain[1]["type"], json!("confirm_action"));
    }

    #[test]
    fn trailing_text_delta_after_last_tool_call_is_not_flushed_into_chain() {
        let mut b = ChainBuilder::new();
        b.tool_call("search", &json!({}), None, None);
        b.tool_result("search", "ok");
        b.text_delta("Here is the final answer");
        assert_eq!(b.finish().len(), 1);
    }
}
