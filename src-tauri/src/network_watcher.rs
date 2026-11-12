//! Event-driven detection of the Windows connection cost (metered vs unmetered)
//! through the Network List Manager COM API.
//!
//! A dedicated MTA thread registers a sink for `INetworkCostManagerEvents`
//! (cost changes) and `INetworkListManagerEvents` (connectivity changes, used
//! as a fallback trigger) and then parks. COM delivers callbacks on its own RPC
//! threads, so the watcher costs no CPU while idle.

use std::sync::atomic::{AtomicU8, Ordering};

use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::Networking::NetworkListManager::{
    INetworkCostManager, INetworkCostManagerEvents, INetworkCostManagerEvents_Impl,
    INetworkListManager, INetworkListManagerEvents, INetworkListManagerEvents_Impl,
    NLM_CONNECTION_COST_FIXED, NLM_CONNECTION_COST_OVERDATALIMIT, NLM_CONNECTION_COST_ROAMING,
    NLM_CONNECTION_COST_VARIABLE, NLM_CONNECTIVITY, NLM_SOCKADDR, NetworkListManager,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IConnectionPointContainer,
};
use windows::core::{Interface, Result as WinResult, implement};

use crate::state::NetworkCost;

const UNKNOWN: u8 = 0;
const UNMETERED: u8 = 1;
const METERED: u8 = 2;

fn classify(cost: u32) -> NetworkCost {
    let metered_mask = (NLM_CONNECTION_COST_FIXED.0
        | NLM_CONNECTION_COST_VARIABLE.0
        | NLM_CONNECTION_COST_OVERDATALIMIT.0
        | NLM_CONNECTION_COST_ROAMING.0) as u32;
    if cost & metered_mask != 0 {
        NetworkCost::Metered
    } else {
        NetworkCost::Unmetered
    }
}

fn query_cost(mgr: &INetworkCostManager) -> NetworkCost {
    let mut cost = 0u32;
    // A null destination address queries the machine-wide (preferred interface) cost.
    match unsafe { mgr.GetCost(&mut cost, std::ptr::null()) } {
        Ok(()) => classify(cost),
        Err(_) => NetworkCost::Unmetered,
    }
}

#[implement(INetworkCostManagerEvents, INetworkListManagerEvents)]
struct CostSink {
    cost_mgr: INetworkCostManager,
    tx: UnboundedSender<NetworkCost>,
    last: AtomicU8,
}

impl CostSink {
    /// Re-reads the machine-wide cost and forwards it only when it changed.
    fn publish(&self) {
        let cost = query_cost(&self.cost_mgr);
        let code = match cost {
            NetworkCost::Unmetered => UNMETERED,
            NetworkCost::Metered => METERED,
        };
        if self.last.swap(code, Ordering::SeqCst) != code {
            let _ = self.tx.send(cost);
        }
    }
}

impl INetworkCostManagerEvents_Impl for CostSink_Impl {
    fn CostChanged(&self, _newcost: u32, _pdestaddr: *const NLM_SOCKADDR) -> WinResult<()> {
        self.publish();
        Ok(())
    }

    fn DataPlanStatusChanged(&self, _pdestaddr: *const NLM_SOCKADDR) -> WinResult<()> {
        Ok(())
    }
}

impl INetworkListManagerEvents_Impl for CostSink_Impl {
    fn ConnectivityChanged(&self, _newconnectivity: NLM_CONNECTIVITY) -> WinResult<()> {
        self.publish();
        Ok(())
    }
}

/// Starts the watcher thread. The current cost is sent immediately, then on every change.
pub fn spawn(tx: UnboundedSender<NetworkCost>) {
    std::thread::Builder::new()
        .name("network-watcher".into())
        .spawn(move || {
            if let Err(e) = run(tx.clone()) {
                eprintln!("[network-watcher] failed: {e}; assuming unmetered network");
                let _ = tx.send(NetworkCost::Unmetered);
            }
        })
        .expect("failed to spawn network watcher thread");
}

fn run(tx: UnboundedSender<NetworkCost>) -> WinResult<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;

        let nlm: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
        let cost_mgr: INetworkCostManager = nlm.cast()?;
        let cpc: IConnectionPointContainer = nlm.cast()?;

        let sink = CostSink {
            cost_mgr: cost_mgr.clone(),
            tx,
            last: AtomicU8::new(UNKNOWN),
        };
        // Emit the initial state before subscribing.
        sink.publish();

        let cost_events: INetworkCostManagerEvents = sink.into();
        let list_events: INetworkListManagerEvents = cost_events.cast()?;

        let cost_cp = cpc.FindConnectionPoint(&INetworkCostManagerEvents::IID)?;
        let _cost_cookie = cost_cp.Advise(&cost_events)?;
        let list_cp = cpc.FindConnectionPoint(&INetworkListManagerEvents::IID)?;
        let _list_cookie = list_cp.Advise(&list_events)?;

        // Keep COM objects alive for the lifetime of the process; zero CPU while parked.
        loop {
            std::thread::park();
        }
    }
}
