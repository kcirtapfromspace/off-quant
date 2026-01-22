use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use serde_json::json;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, ValueEnum)]
enum Runtime {
    Llama,
    Exo,
}

#[derive(Debug, Parser)]
#[command(name = "off-quant")]
#[command(about = "Local LLM runner", version)]
struct Args {
    #[arg(long, value_enum, default_value_t = Runtime::Llama, env = "RUNTIME")]
    runtime: Runtime,

    #[arg(long, env = "MODEL_PATH")]
    model: Option<String>,

    #[arg(long, env = "LLAMA_CPP_BIN")]
    llama_bin: Option<String>,

    #[arg(long, env = "EXO_URL", default_value = "http://localhost:52415")]
    exo_url: String,

    #[arg(long, env = "GPU_LAYERS")]
    gpu_layers: Option<u32>,

    #[arg(long, env = "THREADS")]
    threads: Option<u32>,

    #[arg(long, env = "TEMP")]
    temp: Option<f32>,

    #[arg(long, env = "TOP_P")]
    top_p: Option<f32>,

    #[arg(long, env = "MAX_TOKENS")]
    max_tokens: Option<u32>,

    #[arg(long, env = "REPEAT_PENALTY")]
    repeat_penalty: Option<f32>,

    #[arg(long, env = "CTX_SIZE")]
    ctx_size: Option<u32>,

    #[arg(required = true, num_args = 1..)]
    prompt: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let prompt = args.prompt.join(" ");

    match args.runtime {
        Runtime::Llama => run_llama(&args, &prompt),
        Runtime::Exo => run_exo(&args, &prompt),
    }
}

fn run_llama(args: &Args, prompt: &str) -> Result<()> {
    let model_path = args
        .model
        .clone()
        .context("MODEL_PATH or --model is required for llama.cpp")?;
    let llama_bin = args
        .llama_bin
        .clone()
        .unwrap_or_else(|| "llama-cli".to_string());
    let gpu_layers = args.gpu_layers.unwrap_or(99);
    let threads = args.threads.unwrap_or(8);

    let mut cmd = Command::new(llama_bin);
    cmd.arg("-m")
        .arg(model_path)
        .arg("-p")
        .arg(prompt)
        .arg("--n-gpu-layers")
        .arg(gpu_layers.to_string())
        .arg("-t")
        .arg(threads.to_string())
        .arg("--color")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(temp) = args.temp {
        cmd.arg("--temp").arg(temp.to_string());
    }
    if let Some(top_p) = args.top_p {
        cmd.arg("--top-p").arg(top_p.to_string());
    }
    if let Some(max_tokens) = args.max_tokens {
        cmd.arg("-n").arg(max_tokens.to_string());
    }
    if let Some(repeat_penalty) = args.repeat_penalty {
        cmd.arg("--repeat-penalty").arg(repeat_penalty.to_string());
    }
    if let Some(ctx_size) = args.ctx_size {
        cmd.arg("-c").arg(ctx_size.to_string());
    }

    let status = cmd.status().context("failed to run llama.cpp")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn run_exo(args: &Args, prompt: &str) -> Result<()> {
    let model = args
        .model
        .clone()
        .context("MODEL_PATH or --model is required for EXO")?;

    let url = format!("{}/v1/chat/completions", args.exo_url.trim_end_matches('/'));

    let mut body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": false,
    });

    if let Some(temp) = args.temp {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = args.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(max_tokens) = args.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .context("failed to call EXO API")?
        .error_for_status()
        .context("EXO API returned an error status")?;

    let payload: serde_json::Value = resp.json().context("invalid EXO response")?;
    let content = payload
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str());

    if let Some(text) = content {
        println!("{text}");
        return Ok(());
    }

    bail!("unexpected EXO response shape: {payload}");
}
