#![feature(let_chains)]
#![feature(os_str_display)]

mod cli;
mod process;
mod tui;

fn main() {
    use clap::Parser;

    let args = cli::Cli::parse();

    if let Err(e) = process::process_command(args) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

// fn foo() {
//     let api_key = "AIzaSyC50AM5rv4JIruYaZQreBWAHet5tyuV84o";
//
//     let config = ureq::Agent::config_builder()
//         .http_status_as_error(false) // Keep your original setting
//         .build();
//     let agent = ureq::Agent::new_with_config(config);
//
//     // Use serde_json to create the JSON payload
//     let payload = json!({
//         "model": "gemini-2.0-flash",
//         "messages": [
//             {"role": "user", "content": "中国的首都是哪里？"}
//         ]
//     });
//
//     let res = agent
//         .post("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions")
//         .header("Content-Type", "application/json")
//         .header("Authorization", format!("Bearer {}", api_key))
//         .send_json(payload)
//         .unwrap() // Use send_json for automatic serialization
//         .into_body()
//         .read_to_string()
//         .unwrap(); //  into_string() is cleaner
//
//     println!("{}", res);
// }
// use serde_json::json;
// use serde_yml::modules::error::new;
// use ureq::{
//     Agent,
//     http::header::AUTHORIZATION,
//     middleware::{Middleware, MiddlewareNext},
// };
//
// #[derive(Debug)]
// struct RequestInspector;
//
// impl Middleware for RequestInspector {
//     fn handle(
//         &self,
//         request: ureq::http::Request<ureq::SendBody>,
//         next: MiddlewareNext,
//     ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
//         // Print the request details (Method, URL, Headers)
//         println!("=== Request ===");
//         println!("Method: {}", request.method());
//         println!("URL: {}", request.uri());
//         println!("Headers:");
//         for (name, value) in request.headers() {
//             println!("  {:?}: {:?}", name, value);
//         }
//
//         next.handle(request)
//     }
// }
//
// fn main() {
//     tracing_subscriber::fmt()
//         .with_max_level(tracing::Level::TRACE)
//         .with_line_number(true)
//         .init();
//
//     let api_key = "sk-ba2842c6abfa4c05bcce6ff33d6a3855"; // Replace with your actual API key
//     let endpoint = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";
//     // let endpoint = "http://www.baidu.com";
//
//     // Use serde_json to create the JSON payload
//     let payload = json!({
//         "model": "qwen-max-latest",
//         "messages": [
//             {"role": "user", "content": "中国的首都是哪里？"}
//         ]
//     });
//
//     // Create an agent with the RequestInspector middleware
//     let agent: Agent = Agent::config_builder()
//         .http_status_as_error(false)
//         .middleware(RequestInspector)
//         .build()
//         .into();
//
//     let res = agent
//         .post(endpoint)
//         .header("Content-Type", "application/json")
//         .header("Authorization", format!("Bearer {}", api_key))
//         .send_json(payload)
//         .unwrap();
//
//     println!("=== Response ===");
//     println!("{:?}", res);
//     println!("body: {}", res.into_body().read_to_string().unwrap());
// }
