//! Revision-checked Outline tools; no autonomous maintenance or model calls.
use std::collections::BTreeMap;
use codex_tools::{JsonSchema, ResponsesApiTool, ToolName, ToolSpec};
use crate::function_tool::FunctionCallError;
use crate::tools::context::{ToolInvocation, ToolPayload, boxed_tool_output};
use crate::tools::registry::{CoreToolRuntime, ToolExecutor};
use codex_protocol::models::{FunctionCallOutputPayload, ResponseInputItem};

pub(crate) struct OutlineHandler { pub(crate) write: bool, pub(crate) document_id: String }
struct Output(String);
impl crate::tools::context::ToolOutput for Output {
    fn log_output(&self) -> String { self.0.clone() }
    fn success_for_logging(&self) -> bool { true }
    fn to_response_item(&self, call_id: &str, _: &ToolPayload) -> ResponseInputItem {
        ResponseInputItem::FunctionCallOutput { call_id: call_id.into(), output: FunctionCallOutputPayload::from_text(self.0.clone()) }
    }
    fn code_mode_result(&self, _: &ToolPayload) -> serde_json::Value { serde_json::from_str(&self.0).unwrap_or(serde_json::Value::String(self.0.clone())) }
}
impl ToolExecutor<ToolInvocation> for OutlineHandler {
    fn tool_name(&self) -> ToolName { ToolName::plain(if self.write { "outline_update" } else { "outline_read" }) }
    fn spec(&self) -> ToolSpec {
        let mut properties = BTreeMap::new();
        let required;
        if self.write {
            for (name, description) in [("expected_revision","Revision last supplied or read"),("old_text","Nonempty passage matching exactly once"),("new_text","Replacement passage"),("content","Deliberate full replacement; omit old_text/new_text")] {
                properties.insert(name.into(), JsonSchema::string(Some(description.into())));
            }
            required = vec!["expected_revision".into()];
        } else {
            properties.insert("offset".into(), JsonSchema::integer(Some("First line, starting at 1".into())));
            properties.insert("limit".into(), JsonSchema::integer(Some("Maximum lines, 1–300; default 150".into())));
            required = vec![];
        }
        ToolSpec::Function(ResponsesApiTool {
            name: if self.write { "outline_update" } else { "outline_read" }.into(),
            description: if self.write { "Update the canonical Outline at meaningful checkpoints. Consolidate changes; prefer one unique old_text/new_text replacement. Requires expected_revision; conflicts require rereading, never blind overwriting. Do not update merely to answer an explanation." } else { "Read omitted Outline lines and their current revision. Do not reread text already supplied in the current reference. Read only sections needed for the next implementation decision or edit." }.into(),
            strict: false, defer_loading: None,
            parameters: JsonSchema::object(properties, Some(required), Some(false.into())), output_schema: None,
        })
    }
    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a> where ToolInvocation: 'a {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = &invocation.payload else { return Err(FunctionCallError::RespondToModel("Expected Outline tool arguments".into())); };
            let mut request: serde_json::Value = serde_json::from_str(arguments).map_err(|e| FunctionCallError::RespondToModel(format!("Invalid Outline arguments: {e}")))?;
            let object = request.as_object_mut().ok_or_else(|| FunctionCallError::RespondToModel("Expected an argument object".into()))?;
            object.insert("id".into(), self.document_id.clone().into());
            object.insert("action".into(), if self.write { "update" } else { "read" }.into());
            let result = invocation.session.outline_request(request).await.map_err(FunctionCallError::RespondToModel)?;
            Ok(boxed_tool_output(Output(result.to_string())))
        })
    }
}
impl CoreToolRuntime for OutlineHandler { fn is_builtin_control_tool(&self) -> bool { true } }
