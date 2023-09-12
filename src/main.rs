#![allow(non_snake_case)]

use std::{process::ExitCode, io::{self, Write, copy}, thread, time::Duration, fs::{self, File}};
use clap::{Arg, Command, ArgAction};
use isahc::{Request, RequestExt, ReadResponseExt, prelude::Configurable};
use http::{StatusCode, Method};
use rand::{Rng, rngs::ThreadRng};
use serde_derive::Deserialize;
use chrono::Utc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USER_AGENT: &str = "Blink/2309051925 CFNetwork/1220.1 Darwin/20.3.0";

#[derive(Debug, Clone)]
struct Header {
  key: String,
  value: String
}

#[derive(Debug, Deserialize)]
struct Media {
  media: Vec<Video>
}

#[derive(Debug, Deserialize)]
struct Video {
  media: String,
  created_at: String,
  network_name: String,
  device_name: String,
  deleted: bool
}

#[derive(Debug, Deserialize)]
struct Login {
  account: AccountLogin,
  auth: Auth
}

#[derive(Debug, Deserialize)]
struct AccountLogin {
  account_id: u64,
  client_id: u64,
  tier: String,
  client_verification_required: bool
}

#[derive(Debug, Deserialize)]
struct Auth {
  token: String
}

#[derive(Debug, Clone, Deserialize)]
struct Homescreen {
  networks: Vec<Network>,
  sync_modules: Vec<SyncModules>
}

#[derive(Debug, Clone, Deserialize)]
struct Network {
  id: u32,
  name: String
}

#[derive(Debug, Clone, Deserialize)]
struct SyncModules {
  id: u32,
  network_id: u32,
  local_storage_status: String
}

#[derive(Debug, Deserialize)]
struct SyncManifestInfo {
  id: u64
}

#[derive(Debug, Deserialize)]
struct SyncManifest {
  manifest_id: String,
  clips: Vec<LocalClips>
}

#[derive(Debug, Deserialize)]
struct LocalClips {
  id: String,
  camera_name: String,
  created_at: String
}

#[derive(Debug, Deserialize)]
struct CommandStatus {
  complete: bool,
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
      .help("Alternative API domain")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("wait")
      .short('w')
      .help("How many seconds to wait in between checks (default=120)")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("since")
      .short('s')
      .help("Download media which has been changed this many minutes ago (default=10)")
      .required(false)
      .action(ArgAction::Set)
      .num_args(1)
    )
    .arg(
      Arg::new("output-folder")
      .short('o')
      .help("A custom output folder location")
      .required(false)
      .num_args(1)
    )
    .arg(
      Arg::new("cloud-media")
      .long("disable-cloud-downloads")
      .help("Don't download videos from the cloud")
      .required(false)
      .action(ArgAction::SetTrue)
    )
    .arg(
      Arg::new("local-media")
      .long("enable-local-downloads")
      .help("Download local videos from all sync-modules")
      .required(false)
      .action(ArgAction::SetTrue)
    )
  .get_matches();
  match cli.args_present() {
    true => {
      let email = cli.get_one::<String>("email").unwrap();
      let password = cli.get_one::<String>("password").unwrap();

      let global_domain = if cli.get_one::<String>("domain").is_some() {
        cli.get_one::<String>("domain").unwrap().to_string()
      } else {
        String::from("rest-prod.immedia-semi.com")
      };

      let wait = if cli.get_one::<u8>("wait").is_some() {
        cli.get_one::<u8>("wait").unwrap()
      } else {
        &120
      };

      let since = if cli.get_one::<String>("since").is_some() {
        cli.get_one::<String>("since").unwrap().parse::<u64>().unwrap()
      } else {
        10
      };

      let download_folder = if cli.get_one::<String>("output-folder").is_some() {
        cli.get_one::<String>("output-folder").unwrap()
      } else {
        "downloads"
      };

      let download_cloud_media = !cli.get_flag("cloud-media");
      let download_local_media = cli.get_flag("local-media");

      let header: Header = Header {
        key: "Content-Type".to_string(),
        value: "application/json".to_string()
      };
      let body: String = format!("{{\"email\":\"{}\",\"password\":\"{}\",\"unique_id\":\"{}\",\"device_identifier\":\"BlinkApp\",\"client_name\":\"Computer\",\"reauth\":\"true\"}}", email, password, gen_uid(16, true));
      loop {
        if let Ok(res) = blink_post(&format!("https://{}/api/v5/account/login", global_domain), header.clone(), None, Some(body.clone())) {
          let json_res = serde_json::from_str::<Login>(&res).unwrap();
          let region_domain = if cli.get_one::<String>("domain").is_some() {
            cli.get_one::<String>("domain").unwrap().to_string()
          } else {
            "rest-".to_owned()+&json_res.account.tier+".immedia-semi.com"
          };
          let auth_header: Header = Header {
            key: "TOKEN-AUTH".to_string(),
            value: json_res.auth.token.clone()
          };

          if json_res.account.client_verification_required {
            print!("Please enter the pin for the device-verification.\n: ");
            let pin = get_input();

            let url = format!("https://{}/api/v4/account/{}/client/{}/pin/verify", region_domain, json_res.account.account_id, json_res.account.client_id);

            let header: Header = Header {
              key: "Content-Type".to_string(),
              value: "application/json".to_string()
            };

            let auth_header: Header = Header {
              key: "TOKEN-AUTH".to_string(),
              value: json_res.auth.token.clone()
            };

            let verify_request = blink_post(&url, header, Some(auth_header.clone()), Some(format!("{{\"pin\": {} }}", pin)));

            if verify_request.is_err() {
              println!("Invalid pin provided. Please try again ...");
            } else {
              println!("Success");
              blink_sync(&region_domain, json_res, auth_header, *wait, since, download_folder, download_cloud_media, download_local_media);
            }
          } else {
            blink_sync(&region_domain, json_res, auth_header, *wait, since, download_folder, download_cloud_media, download_local_media);
            thread::sleep(Duration::from_secs(*wait as u64));
          }
        } else {
          println!("Failed to login. Retrying ...");
          thread::sleep(Duration::from_secs(*wait as u64));
        }
      }
    }
    false => ()
  }

  ExitCode::SUCCESS
}

fn gen_uid(size: usize, uid_format: bool) -> String {
  let mut rng = rand::thread_rng();

  if uid_format {
    let uid = format!(
      "BlinkCCamera_{}-{}-{}-{}-{}",
      gen_random_hex(4, &mut rng),
      gen_random_hex(2, &mut rng),
      gen_random_hex(2, &mut rng),
      gen_random_hex(2, &mut rng),
      gen_random_hex(6, &mut rng)
    );
    uid
  } else {
    gen_random_hex(size, &mut rng)
  }
}

fn gen_random_hex(size: usize, rng: &mut ThreadRng) -> String {
  let mut hex_string = String::new();
  for _ in 0..size {
    let digit: u8 = rng.gen_range(0..16);
    hex_string.push_str(&format!("{:x}", digit));
  }
  hex_string
}

fn blink_sync(regional_domain: &String, session: Login, auth_header: Header, wait: u8, since: u64, download_folder: &str,
  download_cloud_media: bool, download_local_media: bool) {
  loop {
    let current_time = Utc::now();
    println!("Checking at: {}", current_time);

    let timestamp = (current_time - chrono::Duration::minutes(since as i64)).to_rfc3339(); // Just to be safe

    let mut page = 1;
    let mut nothing = true;

    if download_cloud_media {
      loop {
        let url = format!("https://{}/api/v1/accounts/{}/media/changed?since={}&page={}",
        regional_domain, session.account.account_id, timestamp, page);

        match blink_get(&url, auth_header.clone()) {
          Ok(txt) => {
            let vids = serde_json::from_str::<Media>(&txt).unwrap();
          
            if vids.media.is_empty() {
              break;
            }
    
            for video in vids.media {
              let output = format!("./{}/{}_{}_{}.mp4",
              download_folder, video.network_name, video.device_name, video.created_at.replace(':', "-"));
    
              if video.deleted || fs::metadata(output.clone()).is_ok() {
                continue;
              } else {
                nothing = false;
              }
    
              fs::create_dir_all(format!("./{}", download_folder)).unwrap();
    
              let url = format!("https://{}{}", regional_domain, video.media);
              for _ in 1..10 {
                if download_video(&url, auth_header.clone(), &output).is_ok() {
                  break;
                }
              }
            }
    
            page += 1;
          },
          Err(_) => {
            break;
          }
        }
      }
    }

    if download_local_media {
      let url_homescreen = format!("https://{}/api/v3/accounts/{}/homescreen", regional_domain, session.account.account_id);
      match blink_get(&url_homescreen, auth_header.clone()) {
        Ok(res) => {
          let homescreen = serde_json::from_str::<Homescreen>(&res).unwrap();
          
          if homescreen.sync_modules.is_empty() {
            continue;
          }

          for sync_module in homescreen.clone().sync_modules {
            let mut network_name: String = String::from("unkown");
            if sync_module.local_storage_status != *"active" {
              continue;
            } else {
              for network in homescreen.clone().networks {
                if network.id == sync_module.network_id {
                  network_name = network.name;
                  println!("  Currently checking: \"{}\"", network_name);
                  break;
                }
              }
            }

            let url_manifest_id = format!("https://{}/api/v1/accounts/{}/networks/{}/sync_modules/{}/local_storage/manifest/request",
            regional_domain, session.account.account_id, sync_module.network_id, sync_module.id);

            let manifest_info = if let Ok(res) = blink_post(&url_manifest_id, auth_header.clone(), None, None) {
              serde_json::from_str::<SyncManifestInfo>(&res).unwrap()
            } else {
              continue;
            };

            let url_manifest = format!("{}/{}", url_manifest_id, manifest_info.id);

            let full_manifest: SyncManifest;
            loop {
              if let Ok(res) = blink_get(&url_manifest, auth_header.clone()) {
                full_manifest = serde_json::from_str::<SyncManifest>(&res).unwrap();
                break;
              }
              thread::sleep(Duration::from_secs(5));  
            }

            'clips: for video in full_manifest.clips {
              let output = format!("./{}/{}_{}_{}.mp4",
              download_folder, network_name, video.camera_name, video.created_at.replace(':', "-"));
              
              fs::create_dir_all(format!("./{}", download_folder)).unwrap();
              if fs::metadata(output.clone()).is_ok() {
                continue;
              } else {
                nothing = false;
              }
              
              // The clip has to be uploaded to blink's servers first to download it.
              let url_clip = format!("https://{}/api/v1/accounts/{}/networks/{}/sync_modules/{}/local_storage/manifest/{}/clip/request/{}",
              regional_domain, session.account.account_id, sync_module.network_id, sync_module.id, full_manifest.manifest_id, video.id);
              'upload: for _ in 1..4 {
                if let Ok(res) = blink_post(&url_clip, auth_header.clone(), None, None) {
                  thread::sleep(Duration::from_secs(2));
                  let upload_id = serde_json::from_str::<SyncManifestInfo>(&res).unwrap().id;
                  let url_upload_state = format!("https://{}/network/{}/command/{}", regional_domain, sync_module.network_id, upload_id);
                  for _ in 0..6 {
                    if let Ok(res) = blink_get(&url_upload_state, auth_header.clone()) {
                      let command_status = serde_json::from_str::<CommandStatus>(&res).unwrap();
                      if command_status.complete {
                        if download_video(&url_clip, auth_header.clone(), &output).is_err() {
                          println!("Download Failed. Trying next clip ...");
                        }
                        thread::sleep(Duration::from_secs(2));
                        break 'upload;
                      }
                    }
                    thread::sleep(Duration::from_secs(2));
                  }
                } else {
                  println!("Upload failed. Continuing in 10 seconds ...");
                  thread::sleep(Duration::from_secs(10));
                  break 'clips;
                };
              }
            }
          }
        },
        Err(_) => {
          return;
        }
      }
    }

    if nothing {
      println!("Nothing new to download.");
    } else {
      println!("Done.")
    }

    thread::sleep(Duration::from_secs(wait as u64));
  }
}

fn download_video(url: &String, auth_header: Header, output: &String) -> Result<(), ()>
{
  let request = Request::builder()
    .method(Method::GET)
    .uri(url)
    .header(auth_header.key, auth_header.value)
    .header("User-Agent", USER_AGENT)
    .header("content-type", "application/json")
    .timeout(Duration::from_secs(20))
    .body(()).unwrap()
  .send();

  if request.is_err() {
    return Err(());
  }

  let res = request.unwrap();

  println!("Saving: {:?}", output);

  let mut file = File::create(output).unwrap();
  if copy(&mut res.into_body(), &mut file).is_err() {
    return Err(());
  };
  Ok(())
}


fn blink_get(url: &String, header: Header) -> Result<String, ()> {
  let request = Request::get(url)
    .method(Method::GET)
    .header(header.key, header.value)
    .header("content-type", "application/json")
    .header("User-Agent", USER_AGENT)
    .timeout(Duration::from_secs(20))
    .body(()).unwrap()
  .send();

  if request.is_err() {
    println!("Error: {}", request.unwrap_err());
    return Err(());
  }

  let mut response = request.unwrap();

  match response.status() {
    StatusCode::OK => {
      Ok(response.text().unwrap())
    }
    _ => {
      if ! response.text().unwrap().contains("Manifest command is in process") {
        println!("Error: {}", response.text().unwrap());
      }
      Err(())
    }
  }
}

fn blink_post(url: &String, header: Header, header2: Option<Header>, body: Option<String>) -> Result<String, ()> {
  let mut builder = Request::builder()
    .uri(url)
    .method(Method::POST)
    .header(header.key, header.value)
    .header("User-Agent", USER_AGENT)
    .header("content-type", "application/json")
  .timeout(Duration::from_secs(20));

  if let Some(header) = header2 {
    builder = builder.header(header.key, header.value);
  }

  let request = if let Some(body_str) = body {
    builder.body(body_str).unwrap().send()
  } else {
    builder.body(()).unwrap().send()
  };

  if request.is_err() {
    println!("Error: {}", request.unwrap_err());
    return Err(());
  }

  let mut response = request.unwrap();

  match response.status() {
    StatusCode::OK => {
      Ok(response.text().unwrap())
    }
    _ => {
      println!("Error: {}", response.text().unwrap());
      Err(())
    }
  }
}

fn get_input() -> String {
  io::stdout().flush().expect("Failed to flush stdout");
  let mut input = String::new();
  io::stdin().read_line(&mut input).unwrap();
  input
}
