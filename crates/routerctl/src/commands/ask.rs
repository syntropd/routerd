use crate::cli::AskArgs;
use crate::client::RouterctlClient;
use anyhow::Result;

pub async fn run_ask(
    client: &RouterctlClient,
    args: AskArgs,
    json_output: bool,
) -> Result<()> {
    let prompt = join_prompt(&args.prompt);
    match client
        .chat_completion(&args.model, &prompt, args.max_tokens, args.effort.as_deref())
        .await
    {
        Ok(text) => {
            if json_output {
                let obj = serde_json::json!({
                    "model": args.model,
                    "prompt": prompt,
                    "response": text,
                    "success": true
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                println!("{}", text);
            }
        }
        Err(e) => {
            if json_output {
                let obj = serde_json::json!({
                    "model": args.model,
                    "prompt": prompt,
                    "success": false,
                    "error": e.to_string()
                });
                println!("{}", serde_json::to_string_pretty(&obj)?);
            } else {
                eprintln!("ask failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

/// Question words become one prompt: `ask say hello` asks "say hello".
fn join_prompt(words: &[String]) -> String {
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn prompt_words_join_with_spaces() {
        assert_eq!(join_prompt(&[]), "");
        assert_eq!(
            join_prompt(&["say".to_string(), "hello".to_string()]),
            "say hello"
        );
    }

    #[test]
    fn ask_parses_model_and_words() {
        let cli = Cli::try_parse_from(["routerctl", "ask", "say", "hello"]).unwrap();
        let Commands::Ask(args) = cli.command else {
            panic!("want Ask");
        };
        assert_eq!(args.model, "router:auto");
        assert_eq!(args.max_tokens, 256);
        assert_eq!(args.effort, None);
        assert_eq!(join_prompt(&args.prompt), "say hello");
        let cli = Cli::try_parse_from(["routerctl", "ask", "-m", "router:fast", "-e", "low", "hi"]).unwrap();
        let Commands::Ask(args) = cli.command else {
            panic!("want Ask");
        };
        assert_eq!(args.model, "router:fast");
        assert_eq!(args.effort.as_deref(), Some("low"));
    }
}
