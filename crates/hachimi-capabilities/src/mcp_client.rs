use std::{collections::BTreeMap, sync::Arc};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::{
    McpCallResult, McpClientError, McpHttpClient, McpProgressHandler, McpPrompt, McpPromptResult,
    McpResource, McpResourceContent, McpResourcePage, McpResourceTemplate, McpResourceTemplatePage,
    McpRunCorrelation, McpServerInfo, McpServerRequestHandler, McpStdioClient, McpToolDefinition,
};

#[derive(Debug)]
pub enum McpClientHandle {
    Stdio(Box<McpStdioClient>),
    StreamableHttp(Box<McpHttpClient>),
}

impl McpClientHandle {
    #[must_use]
    pub fn server_info(&self) -> &McpServerInfo {
        match self {
            Self::Stdio(client) => client.server_info(),
            Self::StreamableHttp(client) => client.server_info(),
        }
    }

    pub async fn ping(&self, cancellation: CancellationToken) -> Result<(), McpClientError> {
        match self {
            Self::Stdio(client) => client.ping(cancellation).await,
            Self::StreamableHttp(client) => client.ping(cancellation).await,
        }
    }

    pub async fn list_tools(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpToolDefinition>, McpClientError> {
        match self {
            Self::Stdio(client) => client.list_tools(cancellation).await,
            Self::StreamableHttp(client) => client.list_tools(cancellation).await,
        }
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        match self {
            Self::Stdio(client) => client.call_tool(name, arguments, cancellation).await,
            Self::StreamableHttp(client) => client.call_tool(name, arguments, cancellation).await,
        }
    }

    pub async fn call_tool_with_handler(
        &self,
        name: &str,
        arguments: Value,
        correlation: Option<McpRunCorrelation>,
        handler: Option<Arc<dyn McpServerRequestHandler>>,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        match self {
            Self::Stdio(client) => {
                client
                    .call_tool_with_handler(name, arguments, correlation, handler, cancellation)
                    .await
            }
            Self::StreamableHttp(client) => {
                client
                    .call_tool_with_handler(name, arguments, correlation, handler, cancellation)
                    .await
            }
        }
    }

    pub async fn call_tool_with_handlers(
        &self,
        name: &str,
        arguments: Value,
        correlation: Option<McpRunCorrelation>,
        request_handler: Option<Arc<dyn McpServerRequestHandler>>,
        progress_handler: Option<Arc<dyn McpProgressHandler>>,
        cancellation: CancellationToken,
    ) -> Result<McpCallResult, McpClientError> {
        match self {
            Self::Stdio(client) => {
                client
                    .call_tool_with_handlers(
                        name,
                        arguments,
                        correlation,
                        request_handler,
                        progress_handler,
                        cancellation,
                    )
                    .await
            }
            Self::StreamableHttp(client) => {
                client
                    .call_tool_with_handlers(
                        name,
                        arguments,
                        correlation,
                        request_handler,
                        progress_handler,
                        cancellation,
                    )
                    .await
            }
        }
    }

    pub async fn list_resources(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResource>, McpClientError> {
        match self {
            Self::Stdio(client) => client.list_resources(cancellation).await,
            Self::StreamableHttp(client) => client.list_resources(cancellation).await,
        }
    }

    pub async fn list_resources_page(
        &self,
        cursor: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<McpResourcePage, McpClientError> {
        match self {
            Self::Stdio(client) => client.list_resources_page(cursor, cancellation).await,
            Self::StreamableHttp(client) => client.list_resources_page(cursor, cancellation).await,
        }
    }

    pub async fn list_resource_templates(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResourceTemplate>, McpClientError> {
        match self {
            Self::Stdio(client) => client.list_resource_templates(cancellation).await,
            Self::StreamableHttp(client) => client.list_resource_templates(cancellation).await,
        }
    }

    pub async fn list_resource_templates_page(
        &self,
        cursor: Option<&str>,
        cancellation: CancellationToken,
    ) -> Result<McpResourceTemplatePage, McpClientError> {
        match self {
            Self::Stdio(client) => {
                client
                    .list_resource_templates_page(cursor, cancellation)
                    .await
            }
            Self::StreamableHttp(client) => {
                client
                    .list_resource_templates_page(cursor, cancellation)
                    .await
            }
        }
    }

    pub async fn read_resource(
        &self,
        uri: &str,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpResourceContent>, McpClientError> {
        match self {
            Self::Stdio(client) => client.read_resource(uri, cancellation).await,
            Self::StreamableHttp(client) => client.read_resource(uri, cancellation).await,
        }
    }

    pub async fn list_prompts(
        &self,
        cancellation: CancellationToken,
    ) -> Result<Vec<McpPrompt>, McpClientError> {
        match self {
            Self::Stdio(client) => client.list_prompts(cancellation).await,
            Self::StreamableHttp(client) => client.list_prompts(cancellation).await,
        }
    }

    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: BTreeMap<String, String>,
        cancellation: CancellationToken,
    ) -> Result<McpPromptResult, McpClientError> {
        match self {
            Self::Stdio(client) => client.get_prompt(name, arguments, cancellation).await,
            Self::StreamableHttp(client) => client.get_prompt(name, arguments, cancellation).await,
        }
    }

    pub async fn shutdown(&self) -> Result<(), McpClientError> {
        match self {
            Self::Stdio(client) => client.shutdown().await,
            Self::StreamableHttp(client) => client.shutdown().await,
        }
    }
}
