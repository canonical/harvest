use anyhow::anyhow;

use super::model::{self, Terminal, TerminalId, TerminalSize};
use super::{store, tmux};
use crate::code::{sessions, CodeEnv, WORKSPACE};
use crate::lxd::console::{Console, ConsoleSpec};
use crate::lxd::containers::Command;

#[derive(Debug)]
pub enum TerminalError {
    NotFound,
    SessionInactive,
    LimitReached,
    Container(anyhow::Error),
}

impl From<anyhow::Error> for TerminalError {
    fn from(error: anyhow::Error) -> Self {
        Self::Container(error)
    }
}

pub async fn create(env: &CodeEnv, session_id: &str) -> Result<Terminal, TerminalError> {
    let container = active_container(env, session_id).await?;
    let prepared = env.lxd.run(&container, Command::shell(&tmux::prepare_script())).await?;
    if !prepared.succeeded() {
        return Err(TerminalError::Container(anyhow!("preparing tmux failed: {}{}", prepared.stdout, prepared.stderr)));
    }
    store::create(&env.db, session_id).await?.ok_or(TerminalError::LimitReached)
}

pub async fn attach(env: &CodeEnv, session_id: &str, id: &TerminalId, size: TerminalSize) -> Result<Console, TerminalError> {
    let container = active_container(env, session_id).await?;
    store::get(&env.db, session_id, id).await?.ok_or(TerminalError::NotFound)?;
    let command = tmux::attach_command(id);
    let environment = model::environment();
    let spec = ConsoleSpec { command: &command, environment: &environment, cwd: WORKSPACE, size: size.into() };
    let console = env.lxd.open_console(&container, spec).await?;
    store::mark_attached(&env.db, id).await?;
    Ok(console)
}

pub async fn end(env: &CodeEnv, session_id: &str, id: &TerminalId) -> Result<(), TerminalError> {
    store::get(&env.db, session_id, id).await?.ok_or(TerminalError::NotFound)?;
    if let Ok(container) = sessions::active_container(env, session_id).await {
        env.lxd.run(&container, Command::shell(&tmux::end_script(id))).await?;
    }
    store::remove(&env.db, session_id, id).await?;
    Ok(())
}

async fn active_container(env: &CodeEnv, session_id: &str) -> Result<String, TerminalError> {
    sessions::active_container(env, session_id)
        .await
        .map_err(|_| TerminalError::SessionInactive)
}
