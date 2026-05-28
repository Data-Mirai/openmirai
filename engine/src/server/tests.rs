use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::tools::builtin::register_all_builtin_tools;
use crate::tools::registry::ToolRegistry;

use super::state::{AppState, LLMFactory};
use super::create_router;

/// Mock LLM factory for unit tests ONLY (#[cfg(test)]).
fn test_llm_factory() -> LLMFactory {
    use crate::adapters::MockLLMResource;
    Arc::new(|| Box::new(MockLLMResource::new()))
}

fn test_app() -> Router {
    let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
    create_router(state)
}

async fn body_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}
    #[tokio::test]
    async fn health_returns_ok() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn version_returns_engine_info() {
        let app = test_app();
        let resp = app
            .oneshot(Request::get("/version").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["version"], "0.4.2");
        assert_eq!(json["engine"], "datamirai-engine-rs");
    }

    #[tokio::test]
    async fn graph_crud_lifecycle() {
        let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
        let app = create_router(state.clone());

        // Create
        let create_body = json!({
            "name": "test-graph",
            "nodes": [{"id": "n1", "tool_type": "ai/llm_call"}],
            "edges": []
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/graphs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp.into_body()).await;
        let graph_id = created["id"].as_str().unwrap().to_string();

        // List
        let resp = app
            .clone()
            .oneshot(Request::get("/api/v1/graphs").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp.into_body()).await;
        assert_eq!(list.as_array().unwrap().len(), 1);

        // Get
        let resp = app
            .clone()
            .oneshot(
                Request::get(&format!("/api/v1/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Delete
        let resp = app
            .clone()
            .oneshot(
                Request::delete(&format!("/api/v1/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Get after delete -> 404
        let resp = app
            .oneshot(
                Request::get(&format!("/api/v1/graphs/{graph_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn agent_crud_lifecycle() {
        let state = AppState::new(ToolRegistry::new(), test_llm_factory(), None);
        let app = create_router(state.clone());

        // First create a graph
        let graph_body = json!({
            "name": "agent-graph",
            "nodes": [{"id": "n1", "tool_type": "ai/llm_call"}],
            "edges": []
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/graphs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&graph_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let created_graph = body_json(resp.into_body()).await;
        let graph_id = created_graph["id"].as_str().unwrap().to_string();

        // Create agent
        let agent_body = json!({
            "name": "test-agent",
            "graph_id": graph_id,
        });
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&agent_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created_agent = body_json(resp.into_body()).await;
        let agent_id = created_agent["id"].as_str().unwrap().to_string();

        // List agents
        let resp = app
            .clone()
            .oneshot(Request::get("/api/v1/agents").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let list = body_json(resp.into_body()).await;
        assert_eq!(list.as_array().unwrap().len(), 1);

        // Get agent
        let resp = app
            .clone()
            .oneshot(
                Request::get(&format!("/api/v1/agents/{agent_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn execute_agent_from_spec() {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let state = AppState::new(registry, test_llm_factory(), None);
        let app = create_router(state.clone());

        // Create agent via from-spec with a simple trigger→response graph
        let spec = json!({
            "name": "test-exec",
            "description": "test",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual", "config": {"payload": {"msg": "hello"}}},
                    {"id": "out", "tool_type": "output/response", "config": {"message": "done"}}
                ],
                "edges": [
                    {"source": "trigger", "target": "out"}
                ]
            }
        });

        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/v1/agents/from-spec")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&spec).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let created = body_json(resp.into_body()).await;
        let agent_id = created["id"].as_str().unwrap().to_string();

        // Execute
        let exec_body = json!({ "trigger_data": {} });
        let resp = app
            .clone()
            .oneshot(
                Request::post(&format!("/api/v1/agents/{agent_id}/execute"))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&exec_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let result = body_json(resp.into_body()).await;
        assert_eq!(result["status"], "Completed");
        assert_eq!(result["agent_id"], agent_id);
    }

    #[tokio::test]
    async fn list_tools_returns_all_builtins() {
        let mut registry = ToolRegistry::new();
        register_all_builtin_tools(&mut registry);
        let state = AppState::new(registry, test_llm_factory(), None);
        let app = create_router(state);

        let resp = app
            .oneshot(Request::get("/api/v1/tools").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let tools = body_json(resp.into_body()).await;
        let arr = tools.as_array().unwrap();
        assert!(arr.len() >= 40, "Expected 40+ tools, got {}", arr.len());
    }

    #[tokio::test]
    async fn create_agent_with_invalid_graph_returns_404() {
        let app = test_app();
        let body = json!({ "name": "orphan", "graph_id": "nonexistent" });
        let resp = app
            .oneshot(
                Request::post("/api/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_nonexistent_session_returns_404() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::get("/api/v1/sessions/ghost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn webhook_returns_received() {
        let app = test_app();
        let body = json!({ "event": "push" });
        let resp = app
            .oneshot(
                Request::post("/webhooks/my-hook")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["received"], true);
        assert_eq!(json["path"], "my-hook");
    }
