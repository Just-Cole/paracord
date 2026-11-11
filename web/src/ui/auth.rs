use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{AuthRequest, AuthResponse, check_password, normalize_username};

use crate::api;
use crate::state::use_ctx;

#[component]
pub fn AuthScreen() -> impl IntoView {
    let ctx = use_ctx();
    let register = RwSignal::new(false);
    let username = RwSignal::new(String::new());
    let display_name = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        if busy.get_untracked() {
            return;
        }
        let creating = register.get_untracked();
        if creating {
            if let Err(message) = normalize_username(&username.get_untracked())
                .map(|_| ())
                .and_then(|_| check_password(&password.get_untracked()))
            {
                problem.set(Some(message.to_string()));
                return;
            }
        }
        let body = AuthRequest {
            username: username.get_untracked(),
            password: password.get_untracked(),
            display_name: creating
                .then(|| display_name.get_untracked())
                .filter(|name| !name.trim().is_empty()),
        };
        busy.set(true);
        problem.set(None);
        spawn_local(async move {
            let path = if creating { "/auth/register" } else { "/auth/login" };
            match api::post::<_, AuthResponse>(path, &body).await {
                Ok(response) => ctx.signed_in(response.user),
                Err(error) => { problem.try_set(Some(error.message)); },
            }
            busy.try_set(false);
        });
    };

    view! {
        <main class="auth">
            <section class="auth-card">
                <div class="brand">
                    <span class="brand-mark">"P"</span>
                    <span class="brand-name">"Paracord"</span>
                </div>
                <h1>{move || if register.get() { "Create your account" } else { "Welcome back" }}</h1>
                <p class="muted">
                    {move || if register.get() {
                        "Pick a username. You can change your display name later."
                    } else {
                        "Sign in to pick up where you left off."
                    }}
                </p>
                <form on:submit=submit>
                    <label>
                        "Username"
                        <input
                            autocomplete="username"
                            autocapitalize="none"
                            spellcheck="false"
                            required
                            maxlength="24"
                            bind:value=username
                        />
                    </label>
                    <Show when=move || register.get()>
                        <label>
                            <span>"Display name " <span class="muted">"(optional)"</span></span>
                            <input maxlength="32" bind:value=display_name/>
                        </label>
                    </Show>
                    <label>
                        "Password"
                        <input
                            type="password"
                            required
                            autocomplete=move || if register.get() { "new-password" } else { "current-password" }
                            bind:value=password
                        />
                    </label>
                    <Show when=move || problem.get().is_some()>
                        <p class="form-error" role="alert">{move || problem.get().unwrap_or_default()}</p>
                    </Show>
                    <button class="primary wide" type="submit" disabled=move || busy.get()>
                        {move || match (register.get(), busy.get()) {
                            (_, true) => "One moment…",
                            (true, false) => "Create account",
                            (false, false) => "Sign in",
                        }}
                    </button>
                </form>
                <p class="switch">
                    {move || if register.get() { "Already have an account? " } else { "New here? " }}
                    <button
                        class="link"
                        type="button"
                        on:click=move |_| {
                            register.update(|r| *r = !*r);
                            problem.set(None);
                        }
                    >
                        {move || if register.get() { "Sign in" } else { "Create an account" }}
                    </button>
                </p>
            </section>
        </main>
    }
}
