use std::time::Duration;

use openraft::error::{
    InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError, Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::types::{Node, ServerId, TypeConfig};

pub const APPEND_PATH: &str = "/internal/quorium/raft/append";
pub const VOTE_PATH: &str = "/internal/quorium/raft/vote";
pub const SNAPSHOT_PATH: &str = "/internal/quorium/raft/snapshot";

#[derive(Clone)]
pub struct HttpNetwork {
    http: reqwest::Client,
}

impl HttpNetwork {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

impl RaftNetworkFactory<TypeConfig> for HttpNetwork {
    type Network = HttpConnection;

    async fn new_client(&mut self, target: ServerId, node: &Node) -> HttpConnection {
        HttpConnection {
            http: self.http.clone(),
            target,
            base: node.addr.trim_end_matches('/').to_string(),
        }
    }
}

pub struct HttpConnection {
    http: reqwest::Client,
    target: ServerId,
    base: String,
}

type RpcResult<T, E> = Result<T, RPCError<ServerId, Node, RaftError<ServerId, E>>>;

impl HttpConnection {
    async fn call<Req, Resp, E>(
        &self,
        path: &str,
        request: &Req,
        ttl: Duration,
    ) -> RpcResult<Resp, E>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
        E: std::error::Error + DeserializeOwned,
    {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .json(request)
            .timeout(ttl)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    RPCError::Unreachable(Unreachable::new(&e))
                } else {
                    RPCError::Network(NetworkError::new(&e))
                }
            })?;
        let response = response
            .error_for_status()
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        let outcome: Result<Resp, RaftError<ServerId, E>> = response
            .json()
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        outcome.map_err(|e| RPCError::RemoteError(RemoteError::new(self.target, e)))
    }
}

impl RaftNetwork<TypeConfig> for HttpConnection {
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        option: RPCOption,
    ) -> RpcResult<AppendEntriesResponse<ServerId>, openraft::error::Infallible> {
        self.call(APPEND_PATH, &rpc, option.hard_ttl()).await
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        option: RPCOption,
    ) -> RpcResult<InstallSnapshotResponse<ServerId>, InstallSnapshotError> {
        self.call(SNAPSHOT_PATH, &rpc, option.hard_ttl()).await
    }

    async fn vote(
        &mut self,
        rpc: VoteRequest<ServerId>,
        option: RPCOption,
    ) -> RpcResult<VoteResponse<ServerId>, openraft::error::Infallible> {
        self.call(VOTE_PATH, &rpc, option.hard_ttl()).await
    }
}
