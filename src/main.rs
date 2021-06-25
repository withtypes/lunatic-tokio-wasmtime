use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    thread,
    time::Duration,
};
use tokio::sync::mpsc;

use anyhow::Result;
use wasmtime::*;

type ModuleId = u64;
type ProcessId = u64;

struct LunaticInner {
    next_module_id: AtomicU64,
    next_process_id: AtomicU64,
    modules: RwLock<HashMap<u64, Module>>,
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
                println!("Got {} from WebAssembly", param);
                println!("my host state is: {:?}", caller.data());
            })
            .unwrap();

        let (sender, mut receiver) = mpsc::unbounded_channel();

        let inner = Arc::new(LunaticInner {
            next_module_id: AtomicU64::new(0),
            next_process_id: AtomicU64::new(0),
            modules: Default::default(),
            engine,
            linker,
        });

        let lunatic = inner.clone();

        let task = async move {
            loop {
                if let Some((module_id, process_id)) = receiver.recv().await {
                    let lunatic = lunatic.clone();
                    tokio::spawn(async move {
                        let module = lunatic
                            .modules
                            .read()
                            .unwrap()
                            .get(&module_id)
                            .cloned()
                            .unwrap();
                        let mut store = Store::new(&lunatic.engine, ());
                        store.add_fuel(10).ok();
                        let instance = lunatic.linker.instantiate_async(&mut store, &module);
                        let instance = instance.await.unwrap();
                        let hello = instance
                            .get_typed_func::<(), (), _>(&mut store, "hello")
                            .unwrap();
                        hello.call_async(&mut store, ()).await.unwrap();
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
        self.inner
            .modules
            .write()
            .map_err(|_| anyhow::anyhow!(""))?
            .insert(id, module);
        Ok(id)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let wat = r#"
        (module
            (import "host" "hello" (func $host_hello (param i32)))

            (func (export "hello")
                i32.const 3
                call $host_hello)
        )
    "#;
    let (mut lunatic, runner) = Lunatic::new();

    // Move lunatic into another thread from which we can spawn new processes
    // and inspect them.
    thread::spawn(move || {
        let module = lunatic.load(wat).unwrap();
        loop {
            let _proc = lunatic.start(module).ok();
            thread::sleep(Duration::from_secs(1));
        }
    });

    runner.await;
    Ok(())
}
