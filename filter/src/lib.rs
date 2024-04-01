use kinode_process_lib::{await_message, call_init, get_blob, http, println, Address, Message};
use std::collections::HashMap;

use llm_interface::api::openai::{spawn_openai_pkg, OpenaiApi};
use llm_interface::openai::ChatParams;
use llm_interface::openai::Message as OpenaiMessage;

wit_bindgen::generate!({
    path: "wit",
    world: "process",
    exports: {
        world: Component,
    },
});

const PROCESS_ID: &str = "filter:filter:template.os";
const OPENAI_API: &str = include_str!("../../pkg/.openai_key");

// TODO: Zena: Is this OK? Look where it's being used.
fn default_headers() -> HashMap<String, String> {
    HashMap::from([
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Access-Control-Allow-Origin".to_string(), "*".to_string()),
        (
            "Access-Control-Allow-Headers".to_string(),
            "Content-Type".to_string(),
        ),
        (
            "Access-Control-Allow-Methods".to_string(),
            "GET, POST, OPTIONS".to_string(),
        ),
    ])
}

fn handle_internal_messages() -> anyhow::Result<()> {
    let message = await_message()?;

    if !message.is_request() {
        return Err(anyhow::anyhow!("unexpected Response: {:?}", message));
    }
    Ok(())
}

fn handle_http_messages(message: &Message, api: &OpenaiApi)  {
    if let Message::Request { ref body, .. } = message {
        handle_request(body, api);
    }
}

fn handle_request(body: &[u8], api: &OpenaiApi) -> Option<()> {
    let server_request = http::HttpServerRequest::from_bytes(body).ok()?;
    let http_request = server_request.request()?;
    match http_request.method().ok() {
        Some(http::Method::OPTIONS) => {
            // Handle OPTIONS request by returning the necessary CORS headers
            let _ = http::send_response(http::StatusCode::OK, Some(default_headers()), Vec::new());
            return None;
        }
        Some(http::Method::POST) => {
            let body = get_blob()?;
            let bound_path = http_request.bound_path(Some(PROCESS_ID));
            match bound_path {
                "/send" => {
                    send_tweet(&body.bytes, api);
                }
                _ => {}
            }
        }
        _ => {}
    }
    None
}

fn send_tweet(body: &[u8], api: &OpenaiApi) -> Option<()> {
    let tweets: Vec<String> =
        match serde_json::from_slice::<serde_json::Value>(body).ok()?["tweets"].as_array() {
            Some(tweets) => tweets
                .iter()
                .filter_map(|tweet| tweet.as_str().map(String::from))
                .collect(),
            None => vec![],
        };
    let tweet_results = make_request(&tweets, api).ok()?;
    // assert_eq!(tweets.len(), tweet_results.len(), "Tweets and results length mismatch");

    let response_body =
        serde_json::to_string(&serde_json::json!({ "tweet_results": tweet_results })).ok()?;
    println!("sending tweet results: {}", response_body);
    let _ = http::send_response(
        http::StatusCode::OK,
        Some(default_headers()),
        response_body.as_bytes().to_vec(),
    );
    None
}

fn make_request(tweets: &[String], api: &OpenaiApi) -> anyhow::Result<Vec<bool>> {
    let temp_rules: Vec<String> = vec![
        "Nothing related to tech.".into(),
        "Nothing related to finance.".into(),
    ];
    let content = format!(
        r###"
    I am going to give you a series of tweets, and a series of rules. 

    The rules are: 
    {}

    The tweets are: 
    {}

    For each of the tweets, respond 0 if they break one or more rules, and 1 if they don't break any rules. 
    Do not answer with anything else but 0 or 1. No part of the answer should contain anything but the symbols 0 or 1.
    The tweets are delimited by |||.
    "###,
        temp_rules.join("\n"),
        tweets.join("|||\n"),
    );
    let system_prompt = OpenaiMessage {
        role: "system".into(),
        content: "You are a helpful assistant that will only answer with 0 or 1".into(),
    };
    let test_prompt = OpenaiMessage {
        role: "user".into(),
        content: content.into(),
    };
    let chat_params = create_chat_params(vec![system_prompt, test_prompt]);
    let result = OpenaiApi::chat(&api, chat_params)?.content;
    println!("Openai result: {:?}", result);
    let bools = parse_response_to_bool_array(&result);
    Ok(bools)
}

fn parse_response_to_bool_array(response: &str) -> Vec<bool> {
    response
        .chars()
        .filter_map(|c| match c {
            '1' => Some(true),
            '0' => Some(false),
            _ => None,
        })
        .collect()
}

fn create_chat_params(messages: Vec<OpenaiMessage>) -> ChatParams {
    let chat_params = ChatParams {
        model: "gpt-4-turbo-preview".into(),
        messages,
        max_tokens: Some(100),
        // temperature: Some(0.0),
        ..Default::default()
    };
    chat_params
}

call_init!(init);

fn setup(our: &Address) -> OpenaiApi {
    println!("filter: begin");
    if let Err(e) = http::serve_index_html(&our, "ui", false, true, vec!["/", "/send"]) {
        panic!("Error serving index html: {:?}", e);
    }
    let Ok(api) = spawn_openai_pkg(our.clone(), OPENAI_API) else {
        panic!("Failed to spawn openai pkg");
    };
    api
}

fn init(our: Address) {
    let api = setup(&our);

    while let Ok(message) = await_message() {
        if message.source().node != our.node {
            continue;
        }

        if message.source().process == "http_server:distro:sys" {
            handle_http_messages(&message, &api);
        } else {
            match handle_internal_messages() {
                Ok(()) => {}
                Err(e) => {
                    println!("auctioneer: error: {:?}", e);
                }
            };
        }
    }
}
