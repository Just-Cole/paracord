use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{Community, Conversation, Member, OpenDm};

use super::Avatar;
use super::chat::Chat;
use super::modals::ModalHost;
use crate::api;
use crate::state::{Modal, View, use_ctx};
use crate::util::{avatar_style, initials, now_ms};

#[component]
pub fn Shell() -> impl IntoView {
    let ctx = use_ctx();
    // Drop typing indicators that have gone quiet.
    let pruner = set_interval_with_handle(
        move || {
            let now = now_ms();
            if ctx.typing.with_untracked(|list| list.iter().any(|t| t.until < now)) {
                ctx.typing.update(|list| list.retain(|t| t.until >= now));
            }
        },
        Duration::from_secs(1),
    );
    on_cleanup(move || {
        if let Ok(handle) = pruner {
            handle.clear();
        }
    });
    let in_community = move || ctx.view.get().community().is_some();

    view! {
        <div class="shell" class:with-members=in_community>
            <Rail/>
            <Sidebar/>
            <Chat/>
            <Show when=in_community>
                <MemberList/>
            </Show>
        </div>
        <ModalHost/>
    }
}

#[component]
fn Rail() -> impl IntoView {
    let ctx = use_ctx();
    let home_active = move || matches!(ctx.view.get(), View::Home { .. });
    let dm_unread = move || {
        let unread = ctx.unread.get();
        ctx.dms.get().iter().any(|c| unread.contains(&c.id))
    };

    view! {
        <nav class="rail" aria-label="Communities">
            <button
                class="rail-item home"
                class:active=home_active
                class:unread=dm_unread
                title="Direct messages"
                on:click=move |_| ctx.open(View::Home { conversation: None })
            >
                "P"
            </button>
            <div class="rail-sep"></div>
            <For each=move || ctx.communities.get() key=|c| (c.id.clone(), c.name.clone()) let:community>
                <RailCommunity community/>
            </For>
            <button class="rail-item add" title="Create a community" on:click=move |_| ctx.modal.set(Some(Modal::CreateCommunity))>
                "+"
            </button>
            <button class="rail-item add" title="Find communities" on:click=move |_| ctx.modal.set(Some(Modal::Discover))>
                "⌕"
            </button>
        </nav>
    }
}

#[component]
fn RailCommunity(community: Community) -> impl IntoView {
    let ctx = use_ctx();
    let id = community.id.clone();
    let channel_ids: Vec<String> = community.channels.iter().map(|c| c.id.clone()).collect();
    let active = {
        let id = id.clone();
        move || ctx.view.get().community() == Some(id.as_str())
    };
    let unread = move || {
        let set = ctx.unread.get();
        channel_ids.iter().any(|c| set.contains(c))
    };
    let hue = community.id.bytes().fold(0i64, |acc, b| (acc * 31 + b as i64) % 360);
    view! {
        <button
            class="rail-item"
            class:active=active
            class:unread=unread
            title=community.name.clone()
            style=avatar_style(hue)
            on:click=move |_| ctx.open_community(&id)
        >
            {initials(&community.name)}
        </button>
    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let ctx = use_ctx();
    view! {
        <aside class="sidebar">
            {move || match ctx.current_community() {
                Some(community) => view! { <CommunityPanel community/> }.into_any(),
                None => view! { <DmPanel/> }.into_any(),
            }}
            <UserPanel/>
        </aside>
    }
}

#[component]
fn CommunityPanel(community: Community) -> impl IntoView {
    let ctx = use_ctx();
    let is_owner = community.owner_id == ctx.my_id();
    let menu_open = RwSignal::new(false);
    let community_id = community.id.clone();

    let leave = {
        let id = community.id.clone();
        move |_| {
            let id = id.clone();
            menu_open.set(false);
            if !confirm("Leave this community?") {
                return;
            }
            spawn_local(async move {
                match api::post_empty(&format!("/communities/{id}/leave")).await {
                    Ok(()) => ctx.refresh_communities().await,
                    Err(error) => ctx.fail(error),
                }
            });
        }
    };
    let delete = {
        let id = community.id.clone();
        let name = community.name.clone();
        move |_| {
            let id = id.clone();
            menu_open.set(false);
            if !confirm(&format!("Delete {name} and all its messages? This can't be undone.")) {
                return;
            }
            spawn_local(async move {
                match api::delete(&format!("/communities/{id}")).await {
                    Ok(()) => ctx.refresh_communities().await,
                    Err(error) => ctx.fail(error),
                }
            });
        }
    };
    let rename_target = community.clone();

    view! {
        <header class="sidebar-head">
            <button class="sidebar-title" on:click=move |_| menu_open.update(|o| *o = !*o) aria-haspopup="menu">
                <span>{community.name.clone()}</span>
                <span class="chev">{move || if menu_open.get() { "✕" } else { "▾" }}</span>
            </button>
            <Show when=move || menu_open.get()>
                <div class="menu" role="menu">
                    {if is_owner {
                        let target = rename_target.clone();
                        let channel_for = community_id.clone();
                        view! {
                            <button role="menuitem" on:click=move |_| {
                                menu_open.set(false);
                                ctx.modal.set(Some(Modal::NewChannel(channel_for.clone())));
                            }>"Create channel"</button>
                            <button role="menuitem" on:click=move |_| {
                                menu_open.set(false);
                                ctx.modal.set(Some(Modal::RenameCommunity(target.clone())));
                            }>"Rename community"</button>
                            <button role="menuitem" class="danger" on:click=delete.clone()>"Delete community"</button>
                        }.into_any()
                    } else {
                        view! {
                            <button role="menuitem" class="danger" on:click=leave.clone()>"Leave community"</button>
                        }.into_any()
                    }}
                </div>
            </Show>
        </header>
        <div class="sidebar-scroll">
            <div class="section-label">
                <span>"Text channels"</span>
                {is_owner.then(|| {
                    let id = community.id.clone();
                    view! {
                        <button class="icon-btn" title="Create channel" on:click=move |_| ctx.modal.set(Some(Modal::NewChannel(id.clone())))>"+"</button>
                    }
                })}
            </div>
            {community.channels.iter().cloned().map(|channel| {
                let id = channel.id.clone();
                let community_id = community.id.clone();
                let active = {
                    let id = id.clone();
                    move || ctx.view.get().scope() == Some(id.as_str())
                };
                let unread = {
                    let id = id.clone();
                    move || ctx.unread.get().contains(&id)
                };
                let edit_target = channel.clone();
                view! {
                    <div class="channel-row" class:active=active.clone() class:unread=unread>
                        <button class="channel" on:click=move |_| ctx.open(View::Community {
                            community: community_id.clone(),
                            channel: id.clone(),
                        })>
                            <span class="hash">"#"</span>
                            <span class="channel-name">{channel.name.clone()}</span>
                        </button>
                        {is_owner.then(|| view! {
                            <button class="icon-btn channel-edit" title="Edit channel" on:click=move |_| ctx.modal.set(Some(Modal::EditChannel(edit_target.clone())))>"⚙"</button>
                        })}
                    </div>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn DmPanel() -> impl IntoView {
    let ctx = use_ctx();
    view! {
        <header class="sidebar-head">
            <div class="sidebar-title static">"Direct messages"</div>
        </header>
        <div class="sidebar-scroll">
            <div class="section-label">
                <span>"Conversations"</span>
                <button class="icon-btn" title="New message" on:click=move |_| ctx.modal.set(Some(Modal::NewDm))>"+"</button>
            </div>
            <Show when=move || ctx.dms.get().is_empty()>
                <p class="empty-note">"No conversations yet. Start one with + or from a member list."</p>
            </Show>
            <For each=move || ctx.dms.get() key=|c| (c.id.clone(), c.user.display_name.clone()) let:conversation>
                <DmRow conversation/>
            </For>
        </div>
    }
}

#[component]
fn DmRow(conversation: Conversation) -> impl IntoView {
    let ctx = use_ctx();
    let id = conversation.id.clone();
    let active = {
        let id = id.clone();
        move || ctx.view.get().scope() == Some(id.as_str())
    };
    let unread = {
        let id = id.clone();
        move || ctx.unread.get().contains(&id)
    };
    view! {
        <button
            class="dm-row"
            class:active=active
            class:unread=unread
            on:click=move |_| ctx.open(View::Home { conversation: Some(id.clone()) })
        >
            <Avatar user=conversation.user.clone()/>
            <span class="dm-name">{conversation.user.display_name.clone()}</span>
        </button>
    }
}

#[component]
fn UserPanel() -> impl IntoView {
    let ctx = use_ctx();
    view! {
        {move || ctx.me.get().map(|me| view! {
            <footer class="user-panel">
                <Avatar user=me.clone() online=ctx.connected.get()/>
                <div class="user-names">
                    <span class="user-display">{me.display_name.clone()}</span>
                    <span class="user-handle">
                        {move || if ctx.connected.get() { format!("@{}", me.username) } else { "Reconnecting…".to_string() }}
                    </span>
                </div>
                <button class="icon-btn" title="Profile" on:click=move |_| ctx.modal.set(Some(Modal::Profile))>"⚙"</button>
            </footer>
        })}
    }
}

#[component]
fn MemberList() -> impl IntoView {
    let ctx = use_ctx();
    let online = Memo::new(move |_| ctx.members.get().into_iter().filter(|m| m.online).collect::<Vec<_>>());
    let offline = Memo::new(move |_| ctx.members.get().into_iter().filter(|m| !m.online).collect::<Vec<_>>());
    view! {
        <aside class="members" aria-label="Members">
            <div class="section-label"><span>{move || format!("Online — {}", online.get().len())}</span></div>
            <For each=move || online.get() key=|m| (m.user.id.clone(), m.user.display_name.clone()) let:member>
                <MemberRow member/>
            </For>
            <Show when=move || !offline.get().is_empty()>
                <div class="section-label"><span>{move || format!("Offline — {}", offline.get().len())}</span></div>
            </Show>
            <For each=move || offline.get() key=|m| (m.user.id.clone(), m.user.display_name.clone()) let:member>
                <MemberRow member/>
            </For>
        </aside>
    }
}

#[component]
fn MemberRow(member: Member) -> impl IntoView {
    let ctx = use_ctx();
    let is_me = member.user.id == ctx.my_id();
    let username = member.user.username.clone();
    let message = move |_| {
        let username = username.clone();
        spawn_local(async move {
            match api::post::<_, Conversation>("/dms", &OpenDm { username }).await {
                Ok(conversation) => {
                    ctx.refresh_dms().await;
                    ctx.open(View::Home { conversation: Some(conversation.id) });
                }
                Err(error) => ctx.fail(error),
            }
        });
    };
    view! {
        <div class="member" class:offline=!member.online>
            <Avatar user=member.user.clone() online=member.online/>
            <div class="member-names">
                <span class="member-display">{member.user.display_name.clone()}</span>
                <span class="member-handle">{format!("@{}", member.user.username)}</span>
            </div>
            {(!is_me).then(|| view! {
                <button class="icon-btn member-dm" title="Send a direct message" on:click=message>"✉"</button>
            })}
        </div>
    }
}

pub fn confirm(question: &str) -> bool {
    window().confirm_with_message(question).unwrap_or(false)
}
