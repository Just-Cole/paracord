use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{
    Channel, Community, Conversation, CreateChannel, DiscoverCommunity, NameRequest, OpenDm,
    UpdateChannel, UpdateMe, User, channel_slug,
};

use wasm_bindgen::JsCast;

use super::shell::confirm;
use crate::api;
use crate::state::{Modal, View, use_ctx};

#[component]
pub fn ModalHost() -> impl IntoView {
    let ctx = use_ctx();
    let close = move || ctx.modal.set(None);
    let on_key = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            close();
        }
    });
    on_cleanup(move || on_key.remove());

    view! {
        {move || ctx.modal.get().map(|modal| {
            let body = match modal {
                Modal::CreateCommunity => view! { <CreateCommunityForm/> }.into_any(),
                Modal::Discover => view! { <DiscoverList/> }.into_any(),
                Modal::NewChannel(community) => view! { <NewChannelForm community/> }.into_any(),
                Modal::EditChannel(channel) => view! { <EditChannelForm channel/> }.into_any(),
                Modal::RenameCommunity(community) => view! { <RenameCommunityForm community/> }.into_any(),
                Modal::NewDm => view! { <NewDmForm/> }.into_any(),
                Modal::Profile => view! { <ProfileForm/> }.into_any(),
            };
            // Move the keyboard into the dialog's first field when it opens.
            let dialog: NodeRef<leptos::html::Div> = NodeRef::new();
            Effect::new(move |_| {
                let Some(el) = dialog.get() else { return };
                let field = el
                    .query_selector("input, textarea")
                    .ok()
                    .flatten()
                    .and_then(|f| f.dyn_into::<web_sys::HtmlElement>().ok());
                match field {
                    Some(field) => { let _ = field.focus(); }
                    None => { let _ = el.focus(); }
                }
            });
            view! {
                <div class="backdrop" on:click=move |_| close()>
                    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" node_ref=dialog on:click=|ev| ev.stop_propagation()>
                        <button class="icon-btn dialog-close" title="Close" on:click=move |_| close()>"✕"</button>
                        {body}
                    </div>
                </div>
            }
        })}
    }
}

/// Shared state for a one-field form: the value, a busy flag and an error.
#[derive(Clone, Copy)]
struct FormState {
    busy: RwSignal<bool>,
    problem: RwSignal<Option<String>>,
}

impl FormState {
    fn new() -> Self {
        Self {
            busy: RwSignal::new(false),
            problem: RwSignal::new(None),
        }
    }

    /// Runs `work`; closes the dialog on success, shows the error otherwise.
    fn run<F>(self, work: F)
    where
        F: Future<Output = Result<(), api::ApiError>> + 'static,
    {
        if self.busy.get_untracked() {
            return;
        }
        let ctx = use_ctx();
        self.busy.set(true);
        self.problem.set(None);
        spawn_local(async move {
            match work.await {
                Ok(()) => ctx.modal.set(None),
                Err(error) if error.unauthorized() => ctx.fail(error),
                Err(error) => { self.problem.try_set(Some(error.message)); },
            }
            self.busy.try_set(false);
        });
    }
}

#[component]
fn Problem(form: FormState) -> impl IntoView {
    view! {
        <Show when=move || form.problem.get().is_some()>
            <p class="form-error" role="alert">{move || form.problem.get().unwrap_or_default()}</p>
        </Show>
    }
}

#[component]
fn CreateCommunityForm() -> impl IntoView {
    let ctx = use_ctx();
    let name = RwSignal::new(String::new());
    let form = FormState::new();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let body = NameRequest { name: name.get_untracked() };
        form.run(async move {
            let community: Community = api::post("/communities", &body).await?;
            ctx.refresh_communities().await;
            ctx.open_community(&community.id);
            Ok(())
        });
    };
    view! {
        <h2>"Create a community"</h2>
        <p class="muted">"It starts with a #general channel. You can add more later."</p>
        <form on:submit=submit>
            <label>"Name" <input required minlength="2" maxlength="40" autofocus bind:value=name/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button type="button" class="ghost" on:click=move |_| ctx.modal.set(Some(Modal::Discover))>"Find one instead"</button>
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Create"</button>
            </div>
        </form>
    }
}

#[component]
fn DiscoverList() -> impl IntoView {
    let ctx = use_ctx();
    let list = RwSignal::new(None::<Vec<DiscoverCommunity>>);
    spawn_local(async move {
        match api::get::<Vec<DiscoverCommunity>>("/communities/discover").await {
            Ok(found) => { list.try_set(Some(found)); },
            Err(error) => ctx.fail(error),
        }
    });
    let join = move |id: String| {
        spawn_local(async move {
            match api::post_empty(&format!("/communities/{id}/join")).await {
                Ok(()) => {
                    ctx.refresh_communities().await;
                    ctx.modal.set(None);
                    ctx.open_community(&id);
                }
                Err(error) => ctx.fail(error),
            }
        });
    };
    view! {
        <h2>"Find a community"</h2>
        {move || match list.get() {
            None => view! { <p class="muted">"Looking…"</p> }.into_any(),
            Some(found) if found.is_empty() => view! {
                <p class="muted">"You're already in every community. Why not start one?"</p>
                <div class="dialog-actions">
                    <button class="primary" on:click=move |_| ctx.modal.set(Some(Modal::CreateCommunity))>"Create a community"</button>
                </div>
            }.into_any(),
            Some(found) => view! {
                <ul class="discover">
                    {found.into_iter().map(|c| {
                        let id = c.id.clone();
                        view! {
                            <li>
                                <div>
                                    <strong>{c.name}</strong>
                                    <span class="muted">{format!(" · {} member{}", c.member_count, if c.member_count == 1 { "" } else { "s" })}</span>
                                </div>
                                <button class="primary small" on:click=move |_| join(id.clone())>"Join"</button>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            }.into_any(),
        }}
    }
}

#[component]
fn NewChannelForm(community: String) -> impl IntoView {
    let ctx = use_ctx();
    let name = RwSignal::new(String::new());
    let topic = RwSignal::new(String::new());
    let form = FormState::new();
    let preview = move || channel_slug(&name.get()).unwrap_or_default();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let body = CreateChannel {
            name: name.get_untracked(),
            topic: topic.get_untracked(),
        };
        let community = community.clone();
        form.run(async move {
            let channel: Channel = api::post(&format!("/communities/{community}/channels"), &body).await?;
            ctx.refresh_communities().await;
            ctx.open(View::Community { community, channel: channel.id });
            Ok(())
        });
    };
    view! {
        <h2>"Create a channel"</h2>
        <form on:submit=submit>
            <label>"Name" <input required maxlength="32" autofocus bind:value=name placeholder="game-night"/></label>
            <Show when=move || !preview().is_empty()>
                <p class="muted small">"Shows as #" {preview}</p>
            </Show>
            <label><span>"Topic " <span class="muted">"(optional)"</span></span> <input maxlength="200" bind:value=topic/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Create channel"</button>
            </div>
        </form>
    }
}

#[component]
fn EditChannelForm(channel: Channel) -> impl IntoView {
    let ctx = use_ctx();
    let name = RwSignal::new(channel.name.clone());
    let topic = RwSignal::new(channel.topic.clone());
    let form = FormState::new();
    let id = channel.id.clone();
    let submit = {
        let id = id.clone();
        move |ev: SubmitEvent| {
            ev.prevent_default();
            let body = UpdateChannel {
                name: Some(name.get_untracked()),
                topic: Some(topic.get_untracked()),
            };
            let id = id.clone();
            form.run(async move {
                api::patch_empty(&format!("/channels/{id}"), &body).await?;
                ctx.refresh_communities().await;
                Ok(())
            });
        }
    };
    let delete = move |_| {
        if !confirm(&format!("Delete #{} and all its messages? This can't be undone.", channel.name)) {
            return;
        }
        let id = id.clone();
        form.run(async move {
            api::delete(&format!("/channels/{id}")).await?;
            ctx.refresh_communities().await;
            Ok(())
        });
    };
    view! {
        <h2>"Channel settings"</h2>
        <form on:submit=submit>
            <label>"Name" <input required maxlength="32" bind:value=name/></label>
            <label>"Topic" <input maxlength="200" bind:value=topic/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button type="button" class="ghost danger" on:click=delete>"Delete channel"</button>
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Save"</button>
            </div>
        </form>
    }
}

#[component]
fn RenameCommunityForm(community: Community) -> impl IntoView {
    let ctx = use_ctx();
    let name = RwSignal::new(community.name.clone());
    let form = FormState::new();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let body = NameRequest { name: name.get_untracked() };
        let id = community.id.clone();
        form.run(async move {
            api::patch_empty(&format!("/communities/{id}"), &body).await?;
            ctx.refresh_communities().await;
            Ok(())
        });
    };
    view! {
        <h2>"Rename community"</h2>
        <form on:submit=submit>
            <label>"Name" <input required minlength="2" maxlength="40" autofocus bind:value=name/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Save"</button>
            </div>
        </form>
    }
}

#[component]
fn NewDmForm() -> impl IntoView {
    let ctx = use_ctx();
    let username = RwSignal::new(String::new());
    let form = FormState::new();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let body = OpenDm {
            username: username.get_untracked().trim().trim_start_matches('@').to_string(),
        };
        form.run(async move {
            let conversation: Conversation = api::post("/dms", &body).await?;
            ctx.refresh_dms().await;
            ctx.open(View::Home { conversation: Some(conversation.id) });
            Ok(())
        });
    };
    view! {
        <h2>"New message"</h2>
        <p class="muted">"You can message anyone you share a community with."</p>
        <form on:submit=submit>
            <label>"Username" <input required autofocus autocapitalize="none" spellcheck="false" placeholder="@username" bind:value=username/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Open conversation"</button>
            </div>
        </form>
    }
}

#[component]
fn ProfileForm() -> impl IntoView {
    let ctx = use_ctx();
    let me = ctx.me.get_untracked();
    let display_name = RwSignal::new(me.as_ref().map(|u| u.display_name.clone()).unwrap_or_default());
    let form = FormState::new();
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        let body = UpdateMe { display_name: display_name.get_untracked() };
        form.run(async move {
            let user: User = api::patch("/me", &body).await?;
            ctx.me.set(Some(user));
            Ok(())
        });
    };
    view! {
        <h2>"Your profile"</h2>
        <p class="muted">{me.map(|u| format!("Signed in as @{}", u.username)).unwrap_or_default()}</p>
        <form on:submit=submit>
            <label>"Display name" <input required maxlength="32" bind:value=display_name/></label>
            <Problem form/>
            <div class="dialog-actions">
                <button type="button" class="ghost danger" on:click=move |_| ctx.sign_out()>"Sign out"</button>
                <button class="primary" type="submit" disabled=move || form.busy.get()>"Save"</button>
            </div>
        </form>
    }
}
