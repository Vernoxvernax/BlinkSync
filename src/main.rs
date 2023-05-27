#![allow(non_snake_case)]

use std::{process::ExitCode, io::{self, Write, copy}, thread, time::Duration, fs::{self, File}};
use clap::{Arg, Command, ArgAction};
use isahc::{Request, RequestExt, ReadResponseExt};
use http::{StatusCode, Method};
use serde_derive::{Deserialize};
use chrono::Utc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
struct Header
{
  key: String,
  value: String
}

#[derive(Debug, Deserialize)]
struct Media
{
  media: Vec<Video>
}

#[derive(Debug, Deserialize)]
struct Video
{
  media: String,
  created_at: String,
  network_name: String,
  device_name: String,
  deleted: bool
}

#[derive(Debug, Deserialize)]
struct Login
{
  account: AccountLogin,
  auth: Auth
}

#[derive(Debug, Deserialize)]
struct AccountLogin
{
  account_id: u64,
  client_id: u64,
  tier: String,
  client_verification_required: bool
}

#[derive(Debug, Deserialize)]
struct Auth
{
  token: String
}

fn main() -> ExitCode {
  let cli = Command::new("BlinkSync")
    .about("automatically sync all videos to your drive")
    .version(VERSION)
    .author("Vernox Vernax")
    .arg(
      Arg::new("email")
      .help("email")
      .required(true)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("password")
      .help("password")
      .required(true)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("domain")
      .short('d')
      .help("alternative api domain")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("wait")
      .short('w')
      .help("how many seconds to wait in between checks (default=120)")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("since")
      .short('s')
      .help("download media which has been changed this many minutes ago (default=10)")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
  .get_matches();
  match cli.args_present()
  {
    true => {
      let email = cli.get_one::<String>("email").unwrap();
      let password = cli.get_one::<String>("password").unwrap();

      let domain = if cli.get_one::<String>("domain").is_some()
      {
        cli.get_one::<String>("domain").unwrap().to_string()
      }
      else
      {
        String::from("rest-prod.immedia-semi.com")
      };

      let wait = if cli.get_one::<u8>("wait").is_some()
      {
        cli.get_one::<u8>("wait").unwrap()
      }
      else
      {
        &120
      };

      let since = if cli.get_one::<String>("since").is_some()
      {
        cli.get_one::<String>("since").unwrap().parse::<u64>().unwrap()
      }
      else
      {
        10
      };

      let header: Header = Header {
        key: "Content-Type".to_string(),
        value: "application/json".to_string()
      };
      let body: String = format!("{{\"unique_id\": \"00000000-0000-0000-0000-000000000000\", \"password\":\"{}\",\"email\":\"{}\", \"reauth\": \"true\"}}", password, email);

      loop
      {
        if let Ok(res) = blink_post(&domain, "api/v5/account/login", header.clone(), None, body.clone())
        {
          let json_res = serde_json::from_str::<Login>(&res).unwrap();
          let domain = if cli.get_one::<String>("domain").is_some() {
            cli.get_one::<String>("domain").unwrap().to_string()
          }
          else
          {
            "rest-".to_owned()+&json_res.account.tier+".immedia-semi.com"
          };
          let auth_header: Header = Header {
            key: "TOKEN-AUTH".to_string(),
            value: json_res.auth.token.clone()
          };

          if json_res.account.client_verification_required
          {
            print!("Please enter the pin for the device-verification.\n: ");
            let pin = get_input();


            let url = format!("api/v4/account/{}/client/{}/pin/verify", json_res.account.account_id, json_res.account.client_id);
            let header: Header = Header {
              key: "Content-Type".to_string(),
              value: "application/json".to_string()
            };
            let auth_header: Header = Header {
              key: "TOKEN-AUTH".to_string(),
              value: json_res.auth.token.clone()
            };

            let verify = blink_post(&domain, &url, header, Some(auth_header.clone()), format!("{{\"pin\": {} }}", pin));

            if verify.is_err()
            {
              println!("Invalid pin provided. Please try again ...");
            }
            else
            {
              println!("Success");
              blink_sync(domain, json_res, auth_header, *wait, since);
            }
          }
          else
          {
            thread::sleep(Duration::from_secs(*wait as u64));
            blink_sync(domain, json_res, auth_header, *wait, since);
          }
        }
        else
        {
          println!("Login credentials incorrect. Please try again ...");
          return ExitCode::FAILURE;
        }
      }
    }
    false => ()
  }

  ExitCode::SUCCESS
}

fn blink_sync(domain: String, session: Login, auth_header: Header, wait: u8, since: u64)
{
  loop
  {
    let current_time = Utc::now();
    println!("Checking at: {}", current_time);

    let timestamp = (current_time - chrono::Duration::minutes(since as i64)).to_rfc3339(); // Just to be safe

    let mut page = 1;
    let mut nothing = true;
    loop
    {
      let url = format!("https://{}/api/v1/accounts/{}/media/changed?since={}&page={}",
      domain, session.account.account_id, timestamp, page);
      match blink_get(url, auth_header.clone())
      {
        Ok(txt) => {
          let vids = serde_json::from_str::<Media>(&txt).unwrap();
  
          if vids.media.is_empty()
          {
            break;
          }
  
          for video in vids.media
          {
            let output = format!("./downloads/{}_{}_{}.mp4",
            video.network_name, video.device_name, video.created_at);
  
            if video.deleted || fs::metadata(output.clone()).is_ok() {
              continue;
            }
            else
            {
              nothing = false;
            }
  
            fs::create_dir_all("./downloads").unwrap();
  
            let url = format!("https://{}{}", domain, video.media);
            download_video(url, auth_header.clone(), output).unwrap();
          }
  
          page += 1;
        },
        Err(_) => {
          return;
        }
      }
    }

    if nothing
    {
      println!("Nothing new to download.");
    }
    else
    {
      println!("Done.")
    }

    thread::sleep(Duration::from_secs(wait as u64));
  }
}

fn download_video(url: String, auth_header: Header, output: String) -> Result<(), ()>
{
  let request = Request::builder()
    .method(Method::GET)
    .uri(url)
    .header(auth_header.key, auth_header.value)
    .body(()).unwrap()
  .send();

  if request.is_err()
  {
    return Err(());
  }

  let res = request.unwrap();

  println!("Saving: {:?}", output);

  let mut file = File::create(output).unwrap();
  copy(&mut res.into_body(), &mut file).unwrap();
  Ok(())
}


fn blink_get(url: String, header: Header) -> Result<String, ()>
{
  let request = Request::get(url)
    .method(Method::GET)
    .header(header.key, header.value)
  .body(()).unwrap()
  .send();

  if request.is_err()
  {
    println!("Error: {}", request.unwrap_err().to_string());
    return Err(());
  }

  let mut response = request.unwrap();

  match response.status()
  {
    StatusCode::OK => {
      Ok(response.text().unwrap())
    }
    _ => {
      println!("Error: {}", response.text().unwrap());
      Err(())
    }
  }
}

fn blink_post(domain: &str, url: &str, header: Header, header2: Option<Header>, body: String) -> Result<String, ()>
{
  let mut builder = Request::builder()
    .uri(format!("https://{}/{}", domain, url))
    .method(Method::POST)
  .header(header.key, header.value);

  if let Some(header) = header2 {
    builder = builder.header(header.key, header.value);
  }

  let request = builder.body(body).unwrap().send();

  if request.is_err()
  {
    return Err(());
  }

  let mut response = request.unwrap();

  match response.status()
  {
    StatusCode::OK => {
      Ok(response.text().unwrap())
    }
    _ => {
      println!("Error: {}", response.text().unwrap());
      Err(())
    }
  }
}

fn get_input() -> String
{
  io::stdout().flush().expect("Failed to flush stdout");
  let mut input = String::new();
  io::stdin().read_line(&mut input).unwrap();
  input
}
