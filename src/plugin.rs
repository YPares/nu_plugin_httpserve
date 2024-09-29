use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use nu_protocol::{IntoSpanned, LabeledError, PipelineData, Signature, SyntaxShape, Type, Value};
use tokio::runtime::{Builder, Runtime};

pub struct HTTPPlugin {
    pub runtime: Runtime,
}

impl HTTPPlugin {
    pub fn new() -> Self {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");
        HTTPPlugin { runtime }
    }
}

impl Default for HTTPPlugin {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HTTPServe;

impl PluginCommand for HTTPServe {
    type Plugin = HTTPPlugin;

    fn name(&self) -> &str {
        "http serve"
    }

    fn description(&self) -> &str {
        "Serve HTTP requests"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            .required("port", SyntaxShape::Int, "TCP port to bind to")
            .required(
                "closure",
                SyntaxShape::Closure(Some(vec![SyntaxShape::Record(vec![])])),
                "The closure to evaluate for each connection",
            )
            .input_output_type(Type::Any, Type::Any)
    }

    // run -> serve -> serve_connection -> hello -> run_eval
    fn run(
        &self,
        plugin: &HTTPPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let span = call.head;
        let port = call.req(0)?;
        let closure = call.req::<Value>(1)?.into_closure()?.into_spanned(span);

        let (ctrlc_tx, ctrlc_rx) = tokio::sync::watch::channel(false);

        let _guard = engine.register_signal_handler(Box::new(move |_| {
            let _ = ctrlc_tx.send(true);
        }))?;

        plugin.runtime.block_on(async move {
            let res = crate::serve::serve(ctrlc_rx, _guard, engine, span, closure, port).await;
            if let Err(err) = res {
                eprintln!("serve error: {:?}", err);
            }
        });

        let span = call.head;
        let value = Value::string("peace", span);
        let body = PipelineData::Value(value, None);
        return Ok(body);
    }
}
