//! The live connection to the server, with automatic reconnect.

use std::cell::{Cell, RefCell};
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{ClientEvent, ServerEvent, User};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::api;
use crate::state::Ctx;

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
    static WANTED: Cell<bool> = const { Cell::new(false) };
    static FAILURES: Cell<u32> = const { Cell::new(0) };
    /// Bumped on every connect/disconnect so callbacks from old sockets do nothing.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
}

fn socket_url() -> Option<String> {
    let location = window().location();
    let scheme = if location.protocol().ok()? == "https:" { "wss" } else { "ws" };
    Some(format!("{scheme}://{}/ws", location.host().ok()?))
}

pub fn connect(ctx: Ctx) {
    WANTED.set(true);
    let generation = GENERATION.get() + 1;
    GENERATION.set(generation);
    open(ctx, generation, false);
}

pub fn disconnect() {
    WANTED.set(false);
    GENERATION.set(GENERATION.get() + 1);
    SOCKET.with_borrow_mut(|slot| {
        if let Some(ws) = slot.take() {
            let _ = ws.close();
        }
    });
}

pub fn send(event: &ClientEvent) {
    let Ok(text) = serde_json::to_string(event) else {
        return;
    };
    SOCKET.with_borrow(|slot| {
        if let Some(ws) = slot
            && ws.ready_state() == WebSocket::OPEN
        {
            let _ = ws.send_with_str(&text);
        }
    });
}

fn open(ctx: Ctx, generation: u32, reconnecting: bool) {
    let Some(url) = socket_url() else {
        return;
    };
    let Ok(ws) = WebSocket::new(&url) else {
        schedule_retry(ctx, generation);
        return;
    };
    let current = move || GENERATION.get() == generation && WANTED.get();

    let on_open = Closure::<dyn FnMut()>::new(move || {
        if !current() {
            return;
        }
        FAILURES.set(0);
        ctx.connected.set(true);
        if reconnecting {
            ctx.resync();
        }
    });
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if !current() {
            return;
        }
        if let Some(text) = event.data().as_string()
            && let Ok(event) = serde_json::from_str::<ServerEvent>(&text)
        {
            ctx.handle(event);
        }
    });
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_: CloseEvent| {
        if !current() {
            return;
        }
        ctx.connected.set(false);
        SOCKET.with_borrow_mut(|slot| *slot = None);
        schedule_retry(ctx, generation);
    });

    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    // One set of handlers per connection attempt; they're small and short-lived.
    on_open.forget();
    on_message.forget();
    on_close.forget();

    SOCKET.with_borrow_mut(|slot| *slot = Some(ws));
}

fn schedule_retry(ctx: Ctx, generation: u32) {
    let failures = FAILURES.get() + 1;
    FAILURES.set(failures);
    let delay = (500u64 << failures.min(5)).min(15_000);
    set_timeout(
        move || {
            if GENERATION.get() != generation || !WANTED.get() {
                return;
            }
            // If the session ended, stop retrying and show the sign-in screen.
            spawn_local(async move {
                match api::get::<User>("/me").await {
                    Err(error) if error.unauthorized() => ctx.signed_out(),
                    _ => {
                        if GENERATION.get() == generation && WANTED.get() {
                            open(ctx, generation, true);
                        }
                    }
                }
            });
        },
        Duration::from_millis(delay),
    );
}
