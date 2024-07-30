#![allow(non_snake_case)]
use std::{fs::{self, File}, io::{self, copy, Read, Write}, path::Path, process::{exit, ExitCode}, thread, time::Duration};
use clap::{Arg, Command, ArgAction};
use isahc::{http::StatusCode, prelude::Configurable, ReadResponseExt, Request, RequestExt};
use serde_derive::{Deserialize, Serialize};
use chrono::Utc;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = "Blink/2404031508 CFNetwork/1220.1 Darwin/20.3.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PostLogin {
  email: String,
  password: String,
  reauth: bool,
  unique_id: String
}

#[derive(Serialize, Deserialize)]
struct ConfigFile {
  uuid: String
}

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
  account: Account,
  auth: Auth
}

#[derive(Debug, Deserialize)]
struct Account {
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
  manifest_id: u64,
  media: Vec<LocalClips>
}

#[derive(Debug, Deserialize)]
struct LocalClips {
  id: u64,
  device_name: String,
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

      let body = PostLogin {
        email: email.to_string(),
        password: password.to_string(),
        reauth: true,
        unique_id: get_uuid()
      };
      let json_body = serde_json::to_string(&body).unwrap();

      loop {
        if let Ok(res) = blink_post(&format!("https://{}/api/v5/account/login", global_domain), header.clone(), None, Some(json_body.clone())) {
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

            let auth_header: Header = Header {
              key: "TOKEN-AUTH".to_string(),
              value: json_res.auth.token.clone()
            };

            let verify_request = blink_post(&url, header.clone(), Some(auth_header.clone()), Some(format!("{{\"pin\": {} }}", pin)));

            if verify_request.is_err() {
              eprintln!("Invalid pin provided. Please try again ...");
              continue;
            } else {
              println!("Success");
            }
          }

          blink_sync(&region_domain, json_res, auth_header, *wait, since, download_folder, download_cloud_media, download_local_media);
          thread::sleep(Duration::from_secs(*wait as u64));
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

fn get_uuid() -> String {
  let path = Path::new("./config.toml");
  if path.exists() {
    if let Ok(mut file) = File::open(path) {
      let mut content = String::new();
      file.read_to_string(&mut content).unwrap();
      let config: ConfigFile = toml::from_str(&content).unwrap();
      return config.uuid;
    }
  }
  let uuid = uuid::Uuid::new_v4().to_string().to_uppercase();
  if let Ok(mut file) = File::create(path) {
    if let Err(err) = file.write_all(toml::to_string(&ConfigFile{uuid: uuid.clone()}).unwrap().as_bytes()) {
      println!("Failed to write to file: {}", err);
    }
  } else {
    println!("Failed to create file.");
  }
  uuid
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
                match download_video(&url, auth_header.clone(), &output) {
                  Ok(()) => {
                    break;
                  }
                  Err(Some(StatusCode::UNAUTHORIZED)) | Err(Some(StatusCode::NOT_ACCEPTABLE)) => {
                    return;
                  },
                  Err(_) => {
                    continue;
                  }
                }
              }
            }
    
            page += 1;
          },
          Err(Some(StatusCode::UNAUTHORIZED)) | Err(Some(StatusCode::NOT_ACCEPTABLE)) => {
            return;
          },
          Err(_) => {
            break;
          }
        }
      }
    }

    if download_local_media {
      let url_homescreen = format!("https://{}/api/v4/accounts/{}/homescreen", regional_domain, session.account.account_id);
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

            let manifest_info = match blink_post(&url_manifest_id, auth_header.clone(), None, None) {
              Ok(res) => {
                serde_json::from_str::<SyncManifestInfo>(&res).unwrap()
              },
              Err(Some(StatusCode::UNAUTHORIZED)) | Err(Some(StatusCode::NOT_ACCEPTABLE)) => {
                return;
              },
              Err(_) => {
                continue;
              }
            };

            thread::sleep(Duration::from_secs(2));
            
            let url_manifest = format!("{}media/{}", url_manifest_id.trim_end_matches("manifest/request"), manifest_info.id);
            let full_manifest: SyncManifest;
            loop {
              match blink_get(&url_manifest, auth_header.clone()) {
                Ok(res) => {
                  if let Ok(json) = serde_json::from_str::<SyncManifest>(&res) {
                    full_manifest = json;
                    break;
                  } else {
                    eprintln!("Failed to serialize into SyncManifest.");
                    exit(1);
                  }
                },
                Err(None) => {
                  thread::sleep(Duration::from_secs(5));
                  continue;
                }
                Err(_) => {
                  return;
                }
              }
            }

            'clips: for video in full_manifest.media {
              let output = format!("./{}/{}_{}_{}.mp4",
              download_folder, network_name, video.device_name, video.created_at.replace(':', "-"));
              
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
                match blink_post(&url_clip, auth_header.clone(), None, None) {
                  Ok(res) => {
                    thread::sleep(Duration::from_secs(2));
                    let upload_id = serde_json::from_str::<SyncManifestInfo>(&res).unwrap().id;
                    let url_upload_state = format!("https://{}/network/{}/command/{}", regional_domain, sync_module.network_id, upload_id);
                    for _ in 0..6 {
                      if let Ok(res) = blink_get(&url_upload_state, auth_header.clone()) {
                        let command_status = serde_json::from_str::<CommandStatus>(&res).unwrap();
                        if command_status.complete {
                          match download_video(&url_clip, auth_header.clone(), &output) {
                            Err(Some(StatusCode::UNAUTHORIZED)) => {
                              return;
                            },
                            Err(_) => {
                              println!("Download Failed. Trying next clip ...");
                            }
                            _ => ()
                          }
                          thread::sleep(Duration::from_secs(2));
                          break 'upload;
                        }
                      }
                      thread::sleep(Duration::from_secs(2));
                    }
                  },
                  Err(Some(StatusCode::UNAUTHORIZED)) => {
                    return;
                  },
                  Err(_) => {
                    println!("Upload failed. Continuing in 10 seconds ...");
                    thread::sleep(Duration::from_secs(10));
                    break 'clips;
                  }
                }
              }
            }
          }
        },
        Err(Some(StatusCode::UNAUTHORIZED)) | Err(Some(StatusCode::NOT_ACCEPTABLE)) => {
          return;
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

fn download_video(url: &String, auth_header: Header, output: &String) -> Result<(), Option<StatusCode>>
{
  let request = Request::get(url)
    .header(auth_header.key, auth_header.value)
    .header("User-Agent", USER_AGENT)
    .header("content-type", "application/json")
    .timeout(Duration::from_secs(20))
    .body(()).unwrap()
  .send();

  if request.is_err() {
    return Err(None);
  }

  let res = request.unwrap();

  match res.status() {
    StatusCode::OK => {
      println!("Saving: {:?}", output);
      let mut file = File::create(output).unwrap();
      if copy(&mut res.into_body(), &mut file).is_err() {
        return Err(None);
      };
      Ok(())
    },
    StatusCode::UNAUTHORIZED => {
      eprintln!("Session expired. Renewing ...");
      Err(Some(StatusCode::UNAUTHORIZED))
    },
    _ => {
      Err(Some(res.status()))
    }
  }
}


fn blink_get(url: &String, header: Header) -> Result<String, Option<StatusCode>> {
  let request = Request::get(url)
    .header(header.key, header.value)
    .header("Content-Type", "application/json")
    .header("User-Agent", USER_AGENT)
    .timeout(Duration::from_secs(10))
    .body(()).unwrap()
  .send();

  if request.is_err() {
    eprintln!("Error: {}\n \"{}\"", request.unwrap_err(), url);
    return Err(None);
  }

  let mut response = request.unwrap();

  match response.status() {
    StatusCode::OK => {
      Ok(
        if let Ok(txt) = response.text() {
          txt
        } else {
          eprintln!("Error: {}\n \"{}\"", response.text().unwrap_err(), url);
          return Err(None);
        }
      )
    },
    StatusCode::UNAUTHORIZED | StatusCode::BAD_REQUEST => {
      eprintln!("Session expired. Renewing ...");
      Err(Some(response.status()))
    },
    _ => {
      if ! response.text().unwrap().contains("Manifest command is in process") {
        eprintln!("Error: {} \"{}\": {}", response.text().unwrap(), url, response.status());
      }
      Err(None)
    }
  }
}

fn blink_post(url: &String, header: Header, header2: Option<Header>, body: Option<String>) -> Result<String, Option<StatusCode>> {
  let mut builder = Request::post(url)
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
    builder.body(String::new()).unwrap().send()
  };

  if request.is_err() {
    eprintln!("Error: {} \"{}\"", request.unwrap_err(), url);
    return Err(None);
  }

  let mut response = request.unwrap();

  match response.status() {
    StatusCode::OK => {
      Ok(response.text().unwrap())
    },
    StatusCode::UNAUTHORIZED => {
      eprintln!("Session expired. Renewing ...");
      Err(Some(StatusCode::UNAUTHORIZED))
    },
    _ => {
      eprintln!("Error: {} \"{}\": {}", response.text().unwrap(), url, response.status());
      Err(None)
    }
  }
}

fn get_input() -> String {
  io::stdout().flush().expect("Failed to flush stdout");
  let mut input = String::new();
  io::stdin().read_line(&mut input).unwrap();
  input
}
