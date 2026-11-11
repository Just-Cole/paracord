pub mod auth;
pub mod chat;
pub mod modals;
pub mod shell;

use leptos::prelude::*;
use paracord_shared::User;

use crate::state::use_ctx;
use crate::util::{avatar_style, initials};

#[component]
pub fn Avatar(user: User, #[prop(optional)] online: Option<bool>) -> impl IntoView {
    let dot = online.map(|on| {
        view! { <span class="presence" class:online=on></span> }
    });
    view! {
        <span class="avatar" style=avatar_style(user.avatar_hue) title=user.display_name.clone()>
            {initials(&user.display_name)}
            {dot}
        </span>
    }
}

#[component]
pub fn Toast() -> impl IntoView {
    let ctx = use_ctx();
    view! {
        <Show when=move || ctx.toast.get().is_some()>
            <div class="toast" role="alert" on:click=move |_| ctx.toast.set(None)>
                {move || ctx.toast.get().unwrap_or_default()}
            </div>
        </Show>
    }
}
