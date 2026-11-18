//! Drawing the submission card in the browser, and handing it over.
//!
//! The whole job of this file is to replay the list of drawing instructions
//! that `submission::layout` produced on an off-screen canvas, ask the browser
//! for a PNG, and hand it over twice: as a download, and - if the browser
//! allows it - on the clipboard, so that a student can paste it straight into
//! Discord. Where anything goes (every position, every word, every colour) is
//! decided in `submission.rs` and tested there. Nothing in this file decides
//! what the card says.
//!
//! The canvas is **detached**: it is created, drawn on and dropped without
//! ever being put in the page, so nothing flashes on screen and the size is
//! exactly 1200 by 675 whatever the window or the display's pixel ratio.
//!
//! Text is drawn with `fillText`, which hands the string to the browser's own
//! text shaper. That is the reason the picture is made here and not by egui:
//! Thai vowels and tone marks are stacked on their consonants by the shaper,
//! and egui does not stack them.
//!
//! The clipboard is reached through `js_sys::Reflect` and not through
//! `web_sys::Clipboard`, because web-sys keeps the clipboard behind an
//! "unstable" compile flag that would have to be set for the whole build.
//! Reflection asks the page for `navigator.clipboard` and `ClipboardItem` at
//! the moment of use and treats their absence (an older browser, a page that
//! is not a secure context) as an ordinary answer, "not copied".

use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Blob, CanvasRenderingContext2d, HtmlAnchorElement, HtmlCanvasElement, Url};

use crate::submission::{Align, Op, SaveEvent, HEIGHT, MONO_STACK, TEXT_STACK, WIDTH};

/// A font shorthand for `ctx.font`: weight, size, then the stack.
fn font(size: f32, bold: bool, mono: bool) -> String {
    format!(
        "{} {}px {}",
        if bold { 700 } else { 400 },
        size.round().max(1.0),
        if mono { MONO_STACK } else { TEXT_STACK }
    )
}

/// What a JavaScript error says, if it says anything.
fn message_of(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            Reflect::get(error, &"message".into())
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_default()
}

fn failed(what: &str, error: JsValue) -> SaveEvent {
    let detail = message_of(&error);
    SaveEvent::Failed(if detail.is_empty() {
        what.to_owned()
    } else {
        format!("{what}: {detail}")
    })
}

fn gone(what: &str) -> SaveEvent {
    SaveEvent::Failed(what.to_owned())
}

/// A rounded rectangle as a path. Built from arcs so that it works in every
/// browser; `roundRect` is newer than the browsers a classroom still has.
fn rounded(ctx: &CanvasRenderingContext2d, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    ctx.begin_path();
    ctx.move_to(x + r, y);
    let _ = ctx.arc_to(x + w, y, x + w, y + h, r);
    let _ = ctx.arc_to(x + w, y + h, x, y + h, r);
    let _ = ctx.arc_to(x, y + h, x, y, r);
    let _ = ctx.arc_to(x, y, x + w, y, r);
    ctx.close_path();
}

fn path(ctx: &CanvasRenderingContext2d, points: &[(f32, f32)]) {
    ctx.begin_path();
    for (index, (x, y)) in points.iter().enumerate() {
        if index == 0 {
            ctx.move_to(f64::from(*x), f64::from(*y));
        } else {
            ctx.line_to(f64::from(*x), f64::from(*y));
        }
    }
}

fn width_of(ctx: &CanvasRenderingContext2d, text: &str) -> f32 {
    ctx.measure_text(text)
        .map(|metrics| metrics.width() as f32)
        .unwrap_or(0.0)
}

/// Replay `ops` on `ctx`.
fn draw(ctx: &CanvasRenderingContext2d, ops: &[Op]) {
    ctx.set_line_cap("round");
    ctx.set_line_join("round");
    for op in ops {
        match op {
            Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                fill,
                stroke,
            } => {
                rounded(
                    ctx,
                    f64::from(*x),
                    f64::from(*y),
                    f64::from(*w),
                    f64::from(*h),
                    f64::from(*radius),
                );
                if let Some(colour) = fill {
                    ctx.set_fill_style_str(&colour.css());
                    ctx.fill();
                }
                if let Some((colour, width)) = stroke {
                    ctx.set_stroke_style_str(&colour.css());
                    ctx.set_line_width(f64::from(*width));
                    ctx.stroke();
                }
            }
            Op::Text {
                x,
                y,
                text,
                size,
                bold,
                mono,
                align,
                colour,
                fit,
            } => {
                let mut size = *size;
                ctx.set_font(&font(size, *bold, *mono));
                // Too wide: shrink the font to fit, but not below 60%, and
                // let `fillText` squeeze whatever is still too wide.
                let mut squeeze: Option<f64> = None;
                if let Some(room) = fit {
                    let width = width_of(ctx, text);
                    if width > *room && width > 0.0 {
                        size = (size * room / width).max(size * 0.6);
                        ctx.set_font(&font(size, *bold, *mono));
                        if width_of(ctx, text) > *room {
                            squeeze = Some(f64::from(*room));
                        }
                    }
                }
                ctx.set_text_align(match align {
                    Align::Left => "left",
                    Align::Center => "center",
                    Align::Right => "right",
                });
                ctx.set_text_baseline("middle");
                ctx.set_fill_style_str(&colour.css());
                let _ = match squeeze {
                    Some(room) => {
                        ctx.fill_text_with_max_width(text, f64::from(*x), f64::from(*y), room)
                    }
                    None => ctx.fill_text(text, f64::from(*x), f64::from(*y)),
                };
            }
            Op::Line {
                points,
                colour,
                width,
            } => {
                path(ctx, points);
                ctx.set_stroke_style_str(&colour.css());
                ctx.set_line_width(f64::from(*width));
                ctx.stroke();
            }
            Op::Circle {
                x,
                y,
                r,
                fill,
                stroke,
            } => {
                ctx.begin_path();
                let _ = ctx.arc(
                    f64::from(*x),
                    f64::from(*y),
                    f64::from(*r),
                    0.0,
                    std::f64::consts::TAU,
                );
                if let Some(colour) = fill {
                    ctx.set_fill_style_str(&colour.css());
                    ctx.fill();
                }
                if let Some((colour, width)) = stroke {
                    ctx.set_stroke_style_str(&colour.css());
                    ctx.set_line_width(f64::from(*width));
                    ctx.stroke();
                }
            }
            Op::Polygon { points, fill } => {
                path(ctx, points);
                ctx.close_path();
                ctx.set_fill_style_str(&fill.css());
                ctx.fill();
            }
        }
    }
}

/// Make the picture of `ops` and save it as `file_name`.
pub fn save_png(ops: Vec<Op>, file_name: String, emit: impl Fn(SaveEvent) + 'static) {
    let emit: Rc<dyn Fn(SaveEvent)> = Rc::new(emit);
    if let Err(event) = start(&ops, &file_name, emit.clone()) {
        emit(event);
    }
}

fn start(ops: &[Op], file_name: &str, emit: Rc<dyn Fn(SaveEvent)>) -> Result<(), SaveEvent> {
    let window = web_sys::window().ok_or_else(|| gone("ไม่มีหน้าต่างเบราว์เซอร์"))?;
    let document = window
        .document()
        .ok_or_else(|| gone("ไม่มีเอกสารของหน้าเว็บ"))?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|error| failed("สร้างผืนวาดไม่ได้", error))?
        .dyn_into()
        .map_err(|_| gone("สร้างผืนวาดไม่ได้"))?;
    canvas.set_width(WIDTH as u32);
    canvas.set_height(HEIGHT as u32);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|error| failed("เปิดผืนวาดไม่ได้", error))?
        .ok_or_else(|| gone("เบราว์เซอร์ไม่ให้วาดบนผืนวาด"))?
        .dyn_into()
        .map_err(|_| gone("เบราว์เซอร์ไม่ให้วาดบนผืนวาด"))?;
    draw(&ctx, ops);

    let file = file_name.to_owned();
    let on_blob = Closure::once_into_js(move |blob: Option<Blob>| match blob {
        Some(blob) => deliver(blob, file, emit),
        None => emit(gone("เบราว์เซอร์สร้างไฟล์ PNG ไม่ได้")),
    });
    canvas
        .to_blob_with_type(on_blob.unchecked_ref(), "image/png")
        .map_err(|error| failed("สร้างไฟล์ PNG ไม่ได้", error))
}

/// The PNG is ready: download it, then try the clipboard.
fn deliver(blob: Blob, file_name: String, emit: Rc<dyn Fn(SaveEvent)>) {
    if let Err(event) = download(&blob, &file_name) {
        emit(event);
        return;
    }
    emit(SaveEvent::Downloaded);
    wasm_bindgen_futures::spawn_local(async move {
        match copy_to_clipboard(&blob).await {
            Ok(()) => emit(SaveEvent::Copied),
            Err(why) => emit(SaveEvent::NotCopied(why)),
        }
    });
}

/// Hand the file to the browser's downloads through a link that is clicked
/// and removed.
fn download(blob: &Blob, file_name: &str) -> Result<(), SaveEvent> {
    let window = web_sys::window().ok_or_else(|| gone("ไม่มีหน้าต่างเบราว์เซอร์"))?;
    let document = window
        .document()
        .ok_or_else(|| gone("ไม่มีเอกสารของหน้าเว็บ"))?;
    let url = Url::create_object_url_with_blob(blob)
        .map_err(|error| failed("ทำลิงก์ดาวน์โหลดไม่ได้", error))?;
    let link: HtmlAnchorElement = document
        .create_element("a")
        .map_err(|error| failed("ทำลิงก์ดาวน์โหลดไม่ได้", error))?
        .dyn_into()
        .map_err(|_| gone("ทำลิงก์ดาวน์โหลดไม่ได้"))?;
    link.set_href(&url);
    link.set_download(file_name);
    link.set_hidden(true);
    let body = document.body().ok_or_else(|| gone("ไม่มีหน้าเว็บให้วางลิงก์"))?;
    body.append_child(&link)
        .map_err(|error| failed("วางลิงก์ดาวน์โหลดไม่ได้", error))?;
    link.click();
    link.remove();
    // The address is let go a while later: some browsers start the download
    // after the click returns, and an address revoked too soon is a download
    // that never starts.
    let release = Closure::once_into_js(move || {
        let _ = Url::revoke_object_url(&url);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(release.unchecked_ref(), 30_000);
    Ok(())
}

/// `navigator.clipboard.write([new ClipboardItem({"image/png": blob})])`,
/// asked for one step at a time so that each missing piece has its own
/// words.
async fn copy_to_clipboard(blob: &Blob) -> Result<(), String> {
    let window = web_sys::window().ok_or("ไม่มีหน้าต่างเบราว์เซอร์")?;
    let get = |target: &JsValue, name: &str| -> Result<JsValue, String> {
        Reflect::get(target, &name.into()).map_err(|_| format!("อ่าน {name} ไม่ได้"))
    };
    let navigator = get(window.as_ref(), "navigator")?;
    let clipboard = get(&navigator, "clipboard")?;
    if clipboard.is_undefined() || clipboard.is_null() {
        return Err("เบราว์เซอร์นี้ไม่มีคลิปบอร์ดสำหรับหน้านี้ (ต้องเปิดผ่าน 127.0.0.1)".into());
    }
    let item_class: Function = get(window.as_ref(), "ClipboardItem")?
        .dyn_into()
        .map_err(|_| "เบราว์เซอร์นี้คัดลอกรูปไม่ได้ (ไม่มี ClipboardItem)".to_owned())?;

    let record = Object::new();
    Reflect::set(&record, &"image/png".into(), blob)
        .map_err(|_| "ใส่รูปลง ClipboardItem ไม่ได้".to_owned())?;
    let item = Reflect::construct(&item_class, &Array::of1(&record))
        .map_err(|_| "สร้าง ClipboardItem ไม่ได้".to_owned())?;

    let write: Function = get(&clipboard, "write")?
        .dyn_into()
        .map_err(|_| "คลิปบอร์ดของเบราว์เซอร์ไม่มี write".to_owned())?;
    let promise = write
        .call1(&clipboard, &Array::of1(&item))
        .map_err(|error| {
            let why = message_of(&error);
            if why.is_empty() {
                "เบราว์เซอร์ไม่ยอมให้คัดลอก".to_owned()
            } else {
                why
            }
        })?;
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(promise))
        .await
        .map(|_| ())
        .map_err(|error| {
            let why = message_of(&error);
            if why.is_empty() {
                "เบราว์เซอร์ไม่ยอมให้คัดลอก".to_owned()
            } else {
                why
            }
        })
}
