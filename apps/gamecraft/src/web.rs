//! The browser's side: the page's query, fetching a world, the console and the page's status
//! card (`index.html`).

use gamecraft_engine::example_world;
use gamecraft_world::World;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Writes a line to the browser's console.
pub fn console(line: &str) {
    web_sys::console::log_1(&JsValue::from_str(line));
}

/// Sets the page's title.
pub fn set_title(title: &str) {
    if let Some(document) = web_sys::window().and_then(|w| w.document()) {
        document.set_title(title);
    }
}

/// A parameter of the page's address (`?world=...`).
pub fn query(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search)
        .ok()?
        .get(name)
}

/// The page's canvas, `gamecraft_canvas`.
pub fn canvas() -> Option<web_sys::HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id("gamecraft_canvas")?
        .dyn_into()
        .ok()
}

/// A JavaScript error as text.
fn describe(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Calls `window[name](...args)` if the page defines it.
fn call(name: &str, args: &[&str]) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(f) = js_sys::Reflect::get(&window, &JsValue::from_str(name)) else {
        return;
    };
    let Ok(f) = f.dyn_into::<js_sys::Function>() else {
        return;
    };
    let list: js_sys::Array = args.iter().map(|a| JsValue::from_str(a)).collect();
    let _ = f.apply(&JsValue::NULL, &list);
}

/// Shows the page's error card.
pub fn fail(title: &str, detail: &str) {
    call("gcFail", &[title, detail]);
}

/// Hides the page's status card: the game runs.
pub fn ready() {
    call("gcHide", &[]);
}

/// Waits for the page's WebGPU check (`window.__gcReady`); its error says what is missing.
pub async fn wait_for_webgpu() -> Result<(), String> {
    let window = web_sys::window().ok_or("the page has no window")?;
    let ready =
        js_sys::Reflect::get(&window, &JsValue::from_str("__gcReady")).map_err(|e| describe(&e))?;
    if let Ok(promise) = ready.dyn_into::<js_sys::Promise>() {
        JsFuture::from(promise)
            .await
            .map_err(|e| format!("WebGPU is not available: {}", describe(&e)))?;
    }
    Ok(())
}

/// Fetches `url` as text.
async fn fetch_text(url: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or("the page has no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("the world {url} could not be fetched: {}", describe(&e)))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| format!("fetching {url} gave no response"))?;
    if !response.ok() {
        return Err(format!(
            "fetching the world {url} answered {} {}",
            response.status(),
            response.status_text()
        ));
    }
    let text = response
        .text()
        .map_err(|e| format!("the world {url} could not be read: {}", describe(&e)))?;
    JsFuture::from(text)
        .await
        .map_err(|e| format!("the world {url} could not be read: {}", describe(&e)))?
        .as_string()
        .ok_or_else(|| format!("the world {url} is not text"))
}

/// Fetches `url` as bytes.
async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("the page has no window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{url} could not be fetched: {}", describe(&e)))?;
    let response: web_sys::Response = response
        .dyn_into()
        .map_err(|_| format!("fetching {url} gave no response"))?;
    if !response.ok() {
        return Err(format!(
            "fetching {url} answered {} {}",
            response.status(),
            response.status_text()
        ));
    }
    let buffer = response
        .array_buffer()
        .map_err(|e| format!("{url} could not be read: {}", describe(&e)))?;
    let buffer = JsFuture::from(buffer)
        .await
        .map_err(|e| format!("{url} could not be read: {}", describe(&e)))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// Where the assets of the page's world are: `?assets=<url>`, else the `assets/` folder beside
/// the file of `?world=<url>` (a world exported into its pack's folder), else none.
fn assets_base() -> Option<String> {
    if let Some(base) = query("assets") {
        return Some(if base.ends_with('/') {
            base
        } else {
            format!("{base}/")
        });
    }
    let world = query("world")?;
    let folder = world.rsplit_once('/').map_or("", |(folder, _)| folder);
    Some(if folder.is_empty() {
        "assets/".to_string()
    } else {
        format!("{folder}/assets/")
    })
}

/// Fetches the files of every model `world` shows, `held_model`s included (a `.gltf` with the
/// files it refers to) from [`assets_base`], measures each model and gives `world` their
/// bounding boxes, as a native
/// host does from a project (so solid models collide in the browser too). A model that cannot
/// be fetched or read is told on the console and left out: the renderer draws its stand-in.
pub async fn load_assets(world: &mut World) -> Vec<(String, Vec<u8>)> {
    let used: std::collections::BTreeSet<String> = world
        .entities()
        .flat_map(|e| {
            let model = e.model().map(|m| m.asset.clone());
            let held = e.held_model().into_iter().flat_map(|h| h.assets().cloned());
            model.into_iter().chain(held)
        })
        .collect();
    let Some(base) = assets_base().filter(|_| !used.is_empty()) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut bounds = std::collections::BTreeMap::new();
    for asset in used {
        let fetched = async {
            let bytes = fetch_bytes(&format!("{base}{asset}")).await?;
            let folder = gamecraft_project::model::folder_of(&asset);
            let mut parts = Vec::new();
            if asset.ends_with(".gltf") {
                for r in gamecraft_project::model::references(&bytes)? {
                    let path = format!("{folder}{r}");
                    let data = fetch_bytes(&format!("{base}{path}")).await?;
                    parts.push((r, path, data));
                }
            }
            let summary = gamecraft_project::inspect(&bytes, &|r: &str| {
                parts
                    .iter()
                    .find(|(name, _, _)| name == r)
                    .map(|(_, _, d)| d.clone())
            })?;
            Ok::<_, String>((bytes, parts, summary))
        }
        .await;
        match fetched {
            Ok((bytes, parts, summary)) => {
                bounds.insert(asset.clone(), summary.bounds);
                files.push((asset, bytes));
                files.extend(parts.into_iter().map(|(_, path, data)| (path, data)));
            }
            Err(e) => console(&format!("The model {asset} is not loaded: {e}")),
        }
    }
    world.set_model_bounds(bounds);
    files
}

/// The world of `?world=<url>`, else the example world.
pub async fn load_world() -> Result<World, String> {
    let Some(url) = query("world") else {
        return Ok(example_world());
    };
    let text = fetch_text(&url).await?;
    World::from_json(&text).map_err(|e| format!("the world {url} is not a valid world: {e}"))
}
