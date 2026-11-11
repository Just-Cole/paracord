mod api;
mod socket;
mod state;
mod ui;
mod util;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::state::Ctx;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx::new();
    provide_context(ctx);
    spawn_local(ctx.check_session());

    view! {
        {move || {
            if !ctx.ready.get() {
                view! { <div class="splash"><div class="splash-mark">"P"</div></div> }.into_any()
            } else if ctx.me.get().is_none() {
                view! { <ui::auth::AuthScreen/> }.into_any()
            } else {
                view! { <ui::shell::Shell/> }.into_any()
            }
        }}
        <ui::Toast/>
    }
}
