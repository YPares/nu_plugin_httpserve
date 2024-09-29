use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use nu_protocol::{IntoSpanned, LabeledError, PipelineData, Signature, SyntaxShape, Type, Value};

pub struct HTTPServePlugin;

impl HTTPServePlugin {
    pub fn new() -> Self {
        HTTPServePlugin {}
    }
}

pub struct HTTPServeCmd;

impl PluginCommand for HTTPServeCmd {
    type Plugin = HTTPServePlugin;

    fn name(&self) -> &str {
        "http serve"
    }

    fn description(&self) -> &str {
        "Serve HTTP requests"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            .required(
                "num_threads",
                SyntaxShape::Int,
                "Number of worker threads to use to reply to incoming connections",
            )
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
        _plugin: &HTTPServePlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let span = call.head;
        let num_threads = call.req(0)?;
        let port = call.req(1)?;
        let closure = call.req::<Value>(2)?.into_closure()?.into_spanned(span);

        let (ctrlc_tx, ctrlc_rx) = tokio::sync::watch::channel(false);

        let _guard = engine.register_signal_handler(Box::new(move |_| {
            let _ = ctrlc_tx.send(true);
        }))?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(num_threads)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime");

        runtime.block_on(async move {
            let res = crate::serve::serve(ctrlc_rx, _guard, engine, span, closure, port).await;
            if let Err(err) = res {
                eprintln!("serve error: {:?}", err);
            }
        });

        println!("http serve finished");

        let span = call.head;
        let value = Value::string("http serve finished", span);
        let body = PipelineData::Value(value, None);
        return Ok(body);
    }
}
