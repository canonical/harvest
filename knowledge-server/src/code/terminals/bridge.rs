use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::{Sink, SinkExt, Stream, StreamExt};
use tokio::time::{interval_at, Instant};

use super::protocol::{lxd_resize, FromBrowser, ToBrowser};

#[derive(Clone, Debug, Default)]
pub struct ActivityFlag(Arc<AtomicBool>);

impl ActivityFlag {
    pub fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

pub struct Signals {
    pub leaving: BoxFuture<'static, ()>,
    pub activity: ActivityFlag,
    pub keepalive: Duration,
}

pub struct BrowserEnd<I, O> {
    pub incoming: I,
    pub outgoing: O,
}

pub struct ShellEnd<O, I, C> {
    pub output: O,
    pub input: I,
    pub control: C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    BrowserLeft,
    ShellClosed,
    NodeLeaving,
}

pub async fn bridge<BI, BO, SO, SI, SC>(
    browser: &mut BrowserEnd<BI, BO>,
    shell: &mut ShellEnd<SO, SI, SC>,
    signals: Signals,
) -> Ended
where
    BI: Stream<Item = FromBrowser> + Unpin,
    BO: Sink<ToBrowser> + Unpin,
    SO: Stream<Item = Vec<u8>> + Unpin,
    SI: Sink<Vec<u8>> + Unpin,
    SC: Sink<String> + Unpin,
{
    let mut keepalive = interval_at(Instant::now() + signals.keepalive, signals.keepalive);
    let mut leaving = signals.leaving;
    loop {
        let ended = tokio::select! {
            message = browser.incoming.next() => forward_to_shell(message, shell, &signals.activity).await,
            output = shell.output.next() => forward_to_browser(output, browser).await,
            _ = keepalive.tick() => send(browser, ToBrowser::Keepalive).await,
            () = &mut leaving => {
                let _ = browser.outgoing.send(ToBrowser::Reconnect).await;
                Some(Ended::NodeLeaving)
            }
        };
        if let Some(ended) = ended {
            return ended;
        }
    }
}

async fn forward_to_shell<SO, SI, SC>(
    message: Option<FromBrowser>,
    shell: &mut ShellEnd<SO, SI, SC>,
    activity: &ActivityFlag,
) -> Option<Ended>
where
    SI: Sink<Vec<u8>> + Unpin,
    SC: Sink<String> + Unpin,
{
    let delivered = match message {
        None => return Some(Ended::BrowserLeft),
        Some(FromBrowser::Input(bytes)) => {
            activity.mark();
            shell.input.send(bytes).await.is_ok()
        }
        Some(FromBrowser::Resize(size)) => shell.control.send(lxd_resize(size)).await.is_ok(),
    };
    (!delivered).then_some(Ended::ShellClosed)
}

async fn forward_to_browser<BI, BO>(output: Option<Vec<u8>>, browser: &mut BrowserEnd<BI, BO>) -> Option<Ended>
where
    BO: Sink<ToBrowser> + Unpin,
{
    match output {
        Some(bytes) => send(browser, ToBrowser::Output(bytes)).await,
        None => Some(Ended::ShellClosed),
    }
}

async fn send<BI, BO>(browser: &mut BrowserEnd<BI, BO>, message: ToBrowser) -> Option<Ended>
where
    BO: Sink<ToBrowser> + Unpin,
{
    browser.outgoing.send(message).await.is_err().then_some(Ended::BrowserLeft)
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc;
    use futures::FutureExt;

    use super::*;
    use crate::code::terminals::model::TerminalSize;

    struct Harness {
        browser_says: mpsc::UnboundedSender<FromBrowser>,
        browser_hears: mpsc::UnboundedReceiver<ToBrowser>,
        shell_says: mpsc::UnboundedSender<Vec<u8>>,
        shell_hears: mpsc::UnboundedReceiver<Vec<u8>>,
        control_hears: mpsc::UnboundedReceiver<String>,
        activity: ActivityFlag,
    }

    fn start(leaving: BoxFuture<'static, ()>, keepalive: Duration) -> (Harness, tokio::task::JoinHandle<Ended>) {
        let (browser_says, incoming) = mpsc::unbounded();
        let (outgoing, browser_hears) = mpsc::unbounded();
        let (shell_says, output) = mpsc::unbounded();
        let (input, shell_hears) = mpsc::unbounded();
        let (control, control_hears) = mpsc::unbounded();
        let activity = ActivityFlag::default();
        let signals = Signals { leaving, activity: activity.clone(), keepalive };
        let task = tokio::spawn(async move {
            let mut browser = BrowserEnd { incoming, outgoing };
            let mut shell = ShellEnd { output, input, control };
            bridge(&mut browser, &mut shell, signals).await
        });
        (Harness { browser_says, browser_hears, shell_says, shell_hears, control_hears, activity }, task)
    }

    fn never() -> BoxFuture<'static, ()> {
        futures::future::pending().boxed()
    }

    #[tokio::test]
    async fn keystrokes_reach_the_shell_and_output_reaches_the_browser() {
        let (mut harness, task) = start(never(), Duration::from_secs(60));

        harness.browser_says.unbounded_send(FromBrowser::Input(b"ls\r".to_vec())).unwrap();
        assert_eq!(harness.shell_hears.next().await, Some(b"ls\r".to_vec()));
        assert!(harness.activity.take());
        assert!(!harness.activity.take());

        harness.shell_says.unbounded_send(b"README.md\r\n".to_vec()).unwrap();
        assert_eq!(harness.browser_hears.next().await, Some(ToBrowser::Output(b"README.md\r\n".to_vec())));

        drop(harness.browser_says);
        assert_eq!(task.await.unwrap(), Ended::BrowserLeft);
    }

    #[tokio::test]
    async fn a_resize_reaches_the_control_socket_without_counting_as_activity() {
        let (mut harness, task) = start(never(), Duration::from_secs(60));

        harness.browser_says.unbounded_send(FromBrowser::Resize(TerminalSize { cols: 100, rows: 30 })).unwrap();
        assert_eq!(harness.control_hears.next().await, Some(lxd_resize(TerminalSize { cols: 100, rows: 30 })));
        assert!(!harness.activity.take());

        drop(harness.shell_says);
        assert_eq!(task.await.unwrap(), Ended::ShellClosed);
    }

    #[tokio::test]
    async fn an_idle_connection_is_kept_alive() {
        let (mut harness, task) = start(never(), Duration::from_millis(20));

        assert_eq!(harness.browser_hears.next().await, Some(ToBrowser::Keepalive));
        task.abort();
    }

    #[tokio::test]
    async fn a_leaving_node_asks_the_browser_to_reconnect_elsewhere() {
        let (leave, left) = futures::channel::oneshot::channel::<()>();
        let (mut harness, task) = start(left.map(|_| ()).boxed(), Duration::from_secs(60));

        leave.send(()).unwrap();
        assert_eq!(harness.browser_hears.next().await, Some(ToBrowser::Reconnect));
        assert_eq!(task.await.unwrap(), Ended::NodeLeaving);
    }
}
