use std::time::Duration;

use axum::extract::ws::WebSocket;
use futures::future::{pending, BoxFuture};
use futures::{FutureExt, SinkExt};

use super::bridge::{bridge, ActivityFlag, Ended, Signals};
use super::protocol::ToBrowser;
use super::sockets;
use crate::cluster::node::{ClusterNode, NodeStanding};
use crate::code::{sessions, CodeEnv};
use crate::lxd::console::Console;

const KEEPALIVE: Duration = Duration::from_secs(30);
const ACTIVITY_EVERY: Duration = Duration::from_secs(60);

pub async fn serve(env: CodeEnv, session_id: String, console: Console, socket: WebSocket, leaving: BoxFuture<'static, ()>) {
    let operation_id = console.operation_id.clone();
    let mut browser = sockets::browser(socket);
    let mut shell = sockets::shell(console);
    let activity = ActivityFlag::default();
    let keeper = tokio::spawn(keep_session_active(env.clone(), session_id.clone(), activity.clone()));

    let ended = bridge(&mut browser, &mut shell, Signals { leaving, activity, keepalive: KEEPALIVE }).await;

    keeper.abort();
    if ended == Ended::ShellClosed {
        let code = env.lxd.console_exit_code(&operation_id).await.ok().flatten();
        let _ = browser.outgoing.send(ToBrowser::Exit(code)).await;
    }
    let _ = browser.outgoing.close().await;
    let _ = shell.input.close().await;
    let _ = shell.control.close().await;
    tracing::info!(session = session_id, ?ended, "terminal connection ended");
}

async fn keep_session_active(env: CodeEnv, session_id: String, activity: ActivityFlag) {
    let mut every = tokio::time::interval(ACTIVITY_EVERY);
    every.tick().await;
    loop {
        every.tick().await;
        if activity.take() {
            if let Err(error) = sessions::touch(&env, &session_id).await {
                tracing::warn!(session = session_id, error = %error, "could not record terminal activity");
            }
        }
    }
}

pub fn node_leaving(node: &ClusterNode) -> BoxFuture<'static, ()> {
    let mut draining = node.watch_draining();
    let mut standing = node.watch_standing();
    async move {
        loop {
            let is_draining = *draining.borrow_and_update();
            let is_fenced = *standing.borrow_and_update() != NodeStanding::Confirmed;
            if is_draining || is_fenced {
                return;
            }
            tokio::select! {
                changed = draining.changed() => if changed.is_err() { pending::<()>().await },
                changed = standing.changed() => if changed.is_err() { pending::<()>().await },
            }
        }
    }
    .boxed()
}
