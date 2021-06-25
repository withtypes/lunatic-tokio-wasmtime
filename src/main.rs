use dashmap::DashMap;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

use anyhow::Result;
use wasmtime::*;

type ModuleId = u64;
type ProcessId = u64;

struct LunaticInner {
    next_module_id: AtomicU64,
    next_process_id: AtomicU64,
    modules: DashMap<u64, Module>,
    started_at: DashMap<u64, Instant>,
    ended_at: DashMap<u64, Instant>,
    instance_pre: DashMap<u64, InstancePre<()>>,
    engine: Engine,
    linker: Linker<()>,
}

struct Lunatic {
    inner: Arc<LunaticInner>,
    sender: mpsc::UnboundedSender<(ModuleId, ProcessId)>,
}

impl Lunatic {
    pub fn new() -> (Self, impl Future<Output = ()>) {
        let mut config = wasmtime::Config::new();
        config.async_support(true).consume_fuel(true);

        let engine = Engine::new(&config).unwrap();
        let mut linker = Linker::new(&engine);

        linker
            .func_wrap("host", "hello", |caller: Caller<'_, ()>, param: i32| {
                //println!("Got {} from WebAssembly", param);
                //println!("my host state is: {:?}", caller.data());
            })
            .unwrap();

        let (sender, mut receiver) = mpsc::unbounded_channel();

        let inner = Arc::new(LunaticInner {
            next_module_id: AtomicU64::new(0),
            next_process_id: AtomicU64::new(0),
            modules: Default::default(),
            instance_pre: Default::default(),
            started_at: Default::default(),
            ended_at: Default::default(),
            engine,
            linker,
        });

        let lunatic = inner.clone();

        let task = async move {
            loop {
                if let Some((module_id, process_id)) = receiver.recv().await {
                    let lunatic = lunatic.clone();
                    tokio::spawn(async move {
                        lunatic.started_at.insert(process_id, Instant::now());
                        let mut store = Store::new(&lunatic.engine, ());
                        store.add_fuel(1000).ok();
                        store.out_of_fuel_async_yield(u32::MAX, 1000);
                        let instance_pre = lunatic.instance_pre.get(&module_id).unwrap();
                        let instance = instance_pre.instantiate_async(&mut store).await.unwrap();
                        let hello = instance
                            .get_typed_func::<u64, u64, _>(&mut store, "hello")
                            .unwrap();
                        let val = hello.call_async(&mut store, process_id).await.unwrap();
                        lunatic.ended_at.insert(process_id, Instant::now());
                    });
                }
            }
        };

        (Self { inner, sender }, task)
    }

    pub fn start(&mut self, module_id: ModuleId) -> Result<ProcessId> {
        let id = self.inner.next_process_id.fetch_add(1, Ordering::Relaxed);
        self.sender.send((module_id, id))?;
        Ok(id)
    }

    pub fn load(&mut self, bytes: impl AsRef<[u8]>) -> Result<ModuleId> {
        let module = Module::new(&self.inner.engine, bytes)?;
        let id = self.inner.next_module_id.fetch_add(1, Ordering::Relaxed);
        self.inner.modules.insert(id, module.clone());
        let mut store = Store::new(&self.inner.engine, ());
        store.add_fuel(1000).ok();
        store.out_of_fuel_async_yield(u32::MAX, 1000);
        let instance_pre = self.inner.linker.instantiate_pre(store, &module).unwrap();
        self.inner.instance_pre.insert(id, instance_pre);
        Ok(id)
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let wat = r#"
        (module
            (import "host" "hello" (func $host_hello (param i32)))

            (func (export "hello")
                i32.const 3
                call $host_hello)
        )
    "#;
    let bytes = include_bytes!("../example/target/wasm32-unknown-unknown/release/lunar.wasm");
    let (mut lunatic, runner) = Lunatic::new();

    // Move lunatic into another thread from which we can spawn new processes
    // and inspect them.
    thread::spawn(move || {
        let _module = lunatic.load(wat).unwrap();
        let module = lunatic.load(bytes).unwrap();
        let n = 3000;
        for _ in 0..n {
            lunatic.start(module).ok();
        }
        loop {
            thread::sleep(Duration::from_secs(1));
            let ended = lunatic.inner.ended_at.len();
            let started = lunatic.inner.started_at.len();
            if ended == n {
                break;
            };
            println!("Ended {}/{}", ended, started);
        }
        let started_at = lunatic
            .inner
            .started_at
            .iter()
            .map(|e| e.value().clone())
            .min()
            .unwrap();
        let ended_at = lunatic
            .inner
            .ended_at
            .iter()
            .map(|e| e.value().clone())
            .max()
            .unwrap();
        let duration = ended_at.checked_duration_since(started_at).unwrap();
        println!("Total duration {}ms", duration.as_millis());
    });

    runner.await;
    Ok(())
}
