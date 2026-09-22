use super::*;
use boa_engine::{
    Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction, Source,
    context::{HostHooks, time::FixedClock},
    js_string,
    module::IdleModuleLoader,
};
use std::{cell::RefCell, rc::Rc};

thread_local! {
    static LOGS: RefCell<(Vec<String>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

struct Limits;
impl HostHooks for Limits {
    fn max_buffer_size(&self, _: &mut Context) -> u64 {
        8 * 1024 * 1024
    }
}

fn log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = args
        .first()
        .unwrap_or(&JsValue::undefined())
        .to_string(context)?
        .to_std_string_escaped();
    LOGS.with(|logs| {
        let mut logs = logs.borrow_mut();
        if logs.0.len() >= 256 || logs.1 + text.len() > 64 * 1024 {
            return Err(JsNativeError::error()
                .with_message("Script log limit exceeded")
                .into());
        }
        logs.1 += text.len();
        logs.0.push(text);
        Ok(JsValue::undefined())
    })
}

// Capture intrinsics before user code, reject lossy values and accessors, then
// serialize a detached tree with null prototypes (including arrays).
const SERIALIZER: &str = r#"(() => {
    const descriptors = Object.getOwnPropertyDescriptors, keys = Reflect.ownKeys;
    const proto = Object.getPrototypeOf, create = Object.create, define = Object.defineProperty;
    const setProto = Object.setPrototypeOf, isArray = Array.isArray;
    const objectProto = Object.prototype, arrayProto = Array.prototype;
    const stringify = JSON.stringify, finite = Number.isFinite, safe = Number.isSafeInteger;
    const fail = () => { throw new TypeError('Script result must be a plain JSON mapping with finite numbers, safe integers, and no cycles or accessors'); };
    return value => {
        const ancestors = [];
        function copy(v, depth) {
            if (depth > 128) fail();
            if (v === null || typeof v === 'string' || typeof v === 'boolean') return v;
            if (typeof v === 'number') { if (!finite(v) || (v % 1 === 0 && !safe(v))) fail(); return v; }
            if (typeof v !== 'object') fail();
            for (let i = 0; i < depth; i++) if (ancestors[i] === v) fail();
            ancestors[depth] = v;
            const array = isArray(v), p = proto(v);
            if (p !== null && p !== (array ? arrayProto : objectProto)) fail();
            const d = descriptors(v), names = keys(d), out = array ? setProto([], null) : create(null);
            if (array) {
                for (let i = 0; i < v.length; i++) {
                    const item = d[i];
                    if (!item || !('value' in item)) fail();
                    define(out, i, {value: copy(item.value, depth + 1), enumerable:true});
                }
            }
            for (let i = 0; i < names.length; i++) {
                const key = names[i], item = d[key];
                if (typeof key !== 'string' || !('value' in item)) fail();
                if (array) { if (key !== 'length' && !(+key >= 0 && +key < v.length && '' + (+key) === key)) fail(); }
                else if (item.enumerable) define(out, key, {value:copy(item.value, depth + 1), enumerable:true});
            }
            return out;
        }
        if (value === null || typeof value !== 'object' || isArray(value)) fail();
        return stringify(copy(value, 0));
    };
})()"#;

pub(super) fn evaluate(request: WorkerRequest) -> WorkerResponse {
    LOGS.with(|logs| *logs.borrow_mut() = (Vec::new(), 0));
    let result = (|| -> Result<serde_json::Value, String> {
        let mut config = request.config;
        for script in request.scripts {
            validate_source(&script).map_err(|e| e.message)?;
            let mut context = Context::builder()
                .module_loader(Rc::new(IdleModuleLoader))
                .host_hooks(Rc::new(Limits))
                .clock(Rc::new(FixedClock::from_millis(0)))
                .can_block(false)
                .build()
                .map_err(|e| e.to_string())?;
            context
                .runtime_limits_mut()
                .set_loop_iteration_limit(10_000_000);
            context.runtime_limits_mut().set_recursion_limit(256);
            context.runtime_limits_mut().set_stack_size_limit(10_240);
            context.runtime_limits_mut().set_backtrace_limit(16);
            let run = (|| -> JsResult<serde_json::Value> {
                context.register_global_builtin_callable(
                    js_string!("__log"),
                    1,
                    NativeFunction::from_fn_ptr(log),
                )?;
                context.eval(Source::from_bytes(r#"(() => {
                    const write = __log, stringify = JSON.stringify;
                    const log = (...args) => write(args.map(x => typeof x === 'string' ? x : stringify(x)).join(' '));
                    globalThis.console = Object.freeze({log, info:log, warn:log, error:log, debug:log});
                })();"#))?;
                let serialize = context
                    .eval(Source::from_bytes(SERIALIZER))?
                    .as_callable()
                    .expect("serializer function");
                let input = JsValue::from_json(&config, &mut context)?;
                let metadata = JsValue::from_json(
                    &serde_json::json!({"profileId":request.id, "apiVersion":1}),
                    &mut context,
                )?;
                context.eval(Source::from_bytes(&script))?;
                let main = context
                    .global_object()
                    .get(js_string!("main"), &mut context)?
                    .as_callable()
                    .ok_or_else(|| {
                        JsNativeError::typ()
                            .with_message("Define function main(config, profileName, context)")
                    })?;
                let output = main.call(
                    &JsValue::undefined(),
                    &[
                        input,
                        JsString::from(request.name.as_str()).into(),
                        metadata,
                    ],
                    &mut context,
                )?;
                let json = serialize
                    .call(&JsValue::undefined(), &[output], &mut context)?
                    .to_string(&mut context)?
                    .to_std_string_escaped();
                if json.len() > OUTPUT_LIMIT / 2 {
                    return Err(JsNativeError::range()
                        .with_message("Script output limit exceeded")
                        .into());
                }
                serde_json::from_str(&json).map_err(|_| {
                    JsNativeError::typ()
                        .with_message("Invalid JSON result")
                        .into()
                })
            })();
            config = run.map_err(|e| e.to_string().chars().take(4096).collect::<String>())?;
        }
        Ok(config)
    })();
    WorkerResponse {
        result,
        logs: LOGS.with(|logs| logs.borrow().0.clone()),
    }
}

pub fn worker_main() {
    let result = (|| -> Result<WorkerResponse, String> {
        let mut input = Vec::new();
        std::io::stdin()
            .take(WIRE_LIMIT as u64 + 1)
            .read_to_end(&mut input)
            .map_err(|e| e.to_string())?;
        if input.len() > WIRE_LIMIT {
            return Err("Script input limit exceeded".into());
        }
        let request =
            serde_json::from_slice(&input).map_err(|e| format!("Invalid worker input: {e}"))?;
        Ok(evaluate(request))
    })();
    let response = result.unwrap_or_else(|error| WorkerResponse {
        result: Err(error),
        logs: Vec::new(),
    });
    let _ = serde_json::to_writer(std::io::stdout().lock(), &response);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(script: &str) -> WorkerResponse {
        evaluate(WorkerRequest {
            scripts: vec![script.into()],
            config: serde_json::json!({"mode":"rule"}),
            name: "Test".into(),
            id: ProfileId::parse("test").unwrap(),
        })
    }
    #[test]
    fn transforms_and_logs_without_async_or_host_access() {
        let response = run(
            "function main(c,n,ctx) { console.log(n); c.name=n; c.id=ctx.profileId; return c; }",
        );
        assert_eq!(response.result.unwrap()["name"], "Test");
        assert_eq!(response.logs, ["Test"]);
        assert!(run("async function main(c) {return c}").result.is_err());
        assert!(
            run("function main(c) {fetch('https://example.com'); return c}")
                .result
                .is_err()
        );
    }
    #[test]
    fn rejects_lossy_results() {
        for expression in [
            "undefined",
            "[]",
            "null",
            "{x:undefined}",
            "{x:NaN}",
            "{x:Infinity}",
            "{x:9007199254740992}",
            "{x:1n}",
            "{x:new Date()}",
            "{get x(){return 1}}",
            "{x:[,1]}",
        ] {
            assert!(
                run(&format!("function main() {{ return {expression}; }}"))
                    .result
                    .is_err(),
                "{expression}"
            );
        }
        assert!(run("function main(c){c.self=c; return c}").result.is_err());
        assert!(
            run("function main(c){while(true){} return c}")
                .result
                .is_err()
        );
    }
    #[test]
    fn serializes_without_user_to_json_or_prototype_hooks() {
        let response = run(
            "function main(c){Object.prototype.toJSON=()=>({bad:true}); Array.prototype.toJSON=()=>null; c.items=[1,2]; return c}",
        );
        assert_eq!(response.result.unwrap()["items"], serde_json::json!([1, 2]));
    }
}
