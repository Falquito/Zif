#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Window, Manager};
use std::fs::File;
use std::path::Path;
use std::time::{Instant, Duration};
use std::io::{BufWriter, BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use sha2::{Sha256, Digest};
use serde_json::json;
use reqwest::Client;

// --- MOTORES DE COMPRESIÓN ---
use zstd::stream::write::Encoder as ZstdEncoder;
use zstd::stream::read::Decoder as ZstdDecoder;
use tar::{Builder, Archive};
use walkdir::WalkDir;
use regex::Regex;

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    percentage: u64,
    current_file: String,
    eta_seconds: u64,
    status: String,
}

// Auxiliar para progreso Zstd
fn emit_progress_throttled(window: &Window, last_emit: &mut Instant, percentage: u64, current_file: &str, start_time: Instant, total_bytes: u64, processed_bytes: u64, status: &str, force: bool) {
    if force || last_emit.elapsed() >= Duration::from_millis(200) {
        let elapsed = start_time.elapsed().as_secs();
        let eta = if processed_bytes > 0 && elapsed > 0 {
            let speed = processed_bytes / elapsed; 
            if speed == 0 { 0 } else { (total_bytes.saturating_sub(processed_bytes)) / speed }
        } else { 0 };

        window.emit("progress", ProgressPayload {
            percentage, current_file: current_file.to_string(), eta_seconds: eta, status: status.to_string(),
        }).unwrap_or(());
        *last_emit = Instant::now();
    }
}

#[tauri::command]
async fn calcular_y_enviar_hash(path: String, id_informe: i32, user_id: i32) -> Result<String, String> {
    let mut file = File::open(&path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];

    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
    }
    
    let hash_hex = format!("{:x}", hasher.finalize());
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("archivo_desconocido")
        .to_string();

    let body = json!({
        "idInforme": id_informe,
        "userId": user_id,
        "dataHashesFilename": [{
            "hash": hash_hex,
            "fileName": file_name
        }]
    });

    let url = "http://localhost:8080/informes/registrar-archivos-hashes";
    let client = Client::new();
    let response = client.post(url).json(&body).send().await.map_err(|e| e.to_string())?;

    if response.status().is_success() {
        Ok(hash_hex)
    } else {
        Err("Error al enviar el hash".to_string())
    }
}

#[tauri::command]
async fn comprimir(window: Window, path: String, destination: String, level: i32) -> Result<String, String> {
    if level >= 22 {
        comprimir_con_7zip(window, path, destination).await
    } else {
        comprimir_con_zstd(window, path, destination, level).await
    }
}

async fn comprimir_con_7zip(window: Window, path: String, destination: String) -> Result<String, String> {
    let mut cmd = Command::new("7z");
    cmd.arg("a").arg("-mx=9").arg("-ms=on").arg("-mmt=on").arg("-bsp1").arg("-y").arg(&destination).arg(&path);
    cmd.stdout(Stdio::piped());
    
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let reader = BufReader::new(child.stdout.take().unwrap());
    let re_percent = Regex::new(r"(\d+)%").unwrap();

    for line in reader.lines() {
        if let Ok(l) = line {
            if let Some(caps) = re_percent.captures(&l) {
                let p = caps[1].parse::<u64>().unwrap_or(0);
                window.emit("progress", ProgressPayload { 
                    percentage: p, current_file: "7-Zip Ultra...".into(), eta_seconds: 0, status: "working".into() 
                }).unwrap_or(());
            }
        }
    }
    child.wait().map_err(|e| e.to_string())?;
    window.emit("progress", ProgressPayload { percentage: 100, current_file: "Terminado".into(), eta_seconds: 0, status: "done".into() }).unwrap();
    Ok(destination)
}

async fn comprimir_con_zstd(window: Window, path: String, destination: String, level: i32) -> Result<String, String> {
    let root_path = Path::new(&path);
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    
    let mut total_size = 0;
    for entry in WalkDir::new(root_path) {
        if let Ok(m) = entry.and_then(|e| e.metadata()) { if m.is_file() { total_size += m.len(); } }
    }

    let file = File::create(&destination).map_err(|e| e.to_string())?;
    let mut encoder = ZstdEncoder::new(BufWriter::new(file), level).map_err(|e| e.to_string())?;
    encoder.multithread(0).unwrap();
    
    let mut tar_builder = Builder::new(encoder);
    let mut processed = 0;

    for entry in WalkDir::new(root_path) {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().is_file() {
            let mut f = File::open(entry.path()).map_err(|e| e.to_string())?;
            let name = entry.path().strip_prefix(root_path.parent().unwrap()).unwrap();
            tar_builder.append_file(name, &mut f).map_err(|e| e.to_string())?;
            processed += entry.metadata().unwrap().len();
            let p = if total_size > 0 { (processed * 100) / total_size } else { 0 };
            emit_progress_throttled(&window, &mut last_emit, p, name.to_str().unwrap(), start_time, total_size, processed, "working", false);
        }
    }
    tar_builder.into_inner().unwrap().finish().unwrap();
    window.emit("progress", ProgressPayload { percentage: 100, current_file: "Terminado".into(), eta_seconds: 0, status: "done".into() }).unwrap();
    Ok(destination)
}

#[tauri::command]
async fn descomprimir(window: Window, tar_path: String, dest_folder: String) -> Result<String, String> {
    if tar_path.ends_with(".7z") {
        Command::new("7z").arg("x").arg("-y").arg(format!("-o{}", dest_folder)).arg(&tar_path).output().unwrap();
    } else {
        let file = File::open(&tar_path).unwrap();
        let decoder = ZstdDecoder::new(file).unwrap();
        Archive::new(decoder).unpack(&dest_folder).unwrap();
    }
    window.emit("progress", ProgressPayload { percentage: 100, current_file: "Listo".into(), eta_seconds: 0, status: "done".into() }).unwrap();
    Ok("OK".into())
}

fn main() {
    tauri::Builder::default()
        // --- PLUGIN SINGLE INSTANCE ---
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            println!("Nueva instancia detectada con args: {:?}", argv);
            // Enviamos los argumentos a la ventana principal para que React los capture
            app.emit_all("deep-link", argv).unwrap();
            
            if let Some(window) = app.get_window("main") {
                let _ = window.set_focus();
                let _ = window.unminimize();
            }
        }))
        .setup(|app| {
            // --- REGISTRO AUTOMÁTICO DEL PROTOCOLO EN WINDOWS ---
            #[cfg(target_os = "windows")]
            {
                use std::path::Path;
                use winreg::enums::*;
                use winreg::RegKey;

                // Registramos zif:// en el sistema de forma silenciosa
                if let Ok(exe_path) = std::env::current_exe() {
                    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
                    let path = Path::new("Software\\Classes\\zif");
                    
                    if let Ok((key, _)) = hkcu.create_subkey(&path) {
                        let _ = key.set_value("", &"URL:zif Protocol");
                        let _ = key.set_value("URL Protocol", &"");
                        
                        if let Ok((cmd_key, _)) = key.create_subkey("shell\\open\\command") {
                            let cmd = format!("\"{}\" \"%1\"", exe_path.display());
                            let _ = cmd_key.set_value("", &cmd);
                        }
                    }
                }
            }
            // ----------------------------------------------------

            let args: Vec<String> = std::env::args().collect();
            let main_window = app.get_window("main").unwrap();

            if args.len() > 1 && args[1].starts_with("zif://") {
                let url = args[1].clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));
                    let _ = main_window.emit("deep-link", vec![url]);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![comprimir, descomprimir, calcular_y_enviar_hash])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}