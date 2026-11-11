use leptos::ev::KeyboardEvent;
use leptos::html::{Div, Textarea};
use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{ClientEvent, ContentRequest, MAX_MESSAGE_LEN, Message, clean_message};

use super::Avatar;
use super::shell::confirm;
use crate::state::{Modal, View, use_ctx};
use crate::util::{clock, day_label, now_ms, same_day, stamp};
use crate::{api, socket};

/// Messages from the same person within this window share one header.
const GROUP_MS: i64 = 5 * 60 * 1000;

#[derive(Clone, PartialEq)]
struct Header {
    title: String,
    topic: String,
    prefix: &'static str,
    placeholder: String,
}

#[component]
pub fn Chat() -> impl IntoView {
    let ctx = use_ctx();

    let header = Memo::new(move |_| match ctx.view.get() {
        View::Community { channel, .. } => ctx.current_community().and_then(|c| {
            c.channels.into_iter().find(|ch| ch.id == channel).map(|ch| Header {
                placeholder: format!("Message #{}", ch.name),
                title: ch.name,
                topic: ch.topic,
                prefix: "#",
            })
        }),
        View::Home { conversation: Some(id) } => ctx
            .dms
            .get()
            .into_iter()
            .find(|c| c.id == id)
            .map(|c| Header {
                placeholder: format!("Message @{}", c.user.display_name),
                title: c.user.display_name,
                topic: format!("@{}", c.user.username),
                prefix: "@",
            }),
        View::Home { conversation: None } => None,
    });

    view! {
        <section class="chat">
            {move || match header.get() {
                Some(header) => view! {
                    <header class="chat-head">
                        <span class="chat-prefix">{header.prefix}</span>
                        <h2>{header.title.clone()}</h2>
                        {(!header.topic.is_empty()).then(|| view! {
                            <span class="chat-topic">{header.topic.clone()}</span>
                        })}
                    </header>
                    <MessageList/>
                    <TypingLine/>
                    <Composer placeholder=header.placeholder.clone()/>
                }.into_any(),
                None => view! { <HomeEmpty/> }.into_any(),
            }}
        </section>
    }
}

#[component]
fn HomeEmpty() -> impl IntoView {
    let ctx = use_ctx();
    view! {
        <div class="home-empty">
            <div class="home-mark">"P"</div>
            <h2>"Direct messages"</h2>
            <p class="muted">"Pick a conversation, or start a new one with someone you share a community with."</p>
            <button class="primary" on:click=move |_| ctx.modal.set(Some(Modal::NewDm))>"New message"</button>
        </div>
    }
}

#[component]
fn MessageList() -> impl IntoView {
    let ctx = use_ctx();
    let scroller: NodeRef<Div> = NodeRef::new();
    // Follow new messages only while the reader is already at the bottom.
    let pinned = StoredValue::new(true);
    // Scroll height before loading older messages, to keep the view still.
    let restore = StoredValue::new(None::<i32>);

    Effect::new(move |_| {
        ctx.messages.track();
        let Some(el) = scroller.get() else { return };
        request_animation_frame(move || {
            if let Some(old_height) = restore.get_value() {
                restore.set_value(None);
                el.set_scroll_top(el.scroll_height() - old_height + el.scroll_top());
            } else if pinned.get_value() {
                el.set_scroll_top(el.scroll_height());
            }
        });
    });
    // A newly opened channel starts at the bottom.
    Effect::new(move |_| {
        ctx.view.track();
        pinned.set_value(true);
    });

    let on_scroll = move |_| {
        if let Some(el) = scroller.get() {
            let gap = el.scroll_height() - el.scroll_top() - el.client_height();
            pinned.set_value(gap < 80);
        }
    };
    let load_older = move |_| {
        if let Some(el) = scroller.get() {
            restore.set_value(Some(el.scroll_height()));
        }
        spawn_local(ctx.load_messages(true));
    };

    view! {
        <div class="messages" node_ref=scroller on:scroll=on_scroll>
            <Show when=move || ctx.has_more.get()>
                <button class="load-older" on:click=load_older disabled=move || ctx.loading.get()>
                    {move || if ctx.loading.get() { "Loading…" } else { "Load older messages" }}
                </button>
            </Show>
            <Show when=move || !ctx.has_more.get() && !ctx.loading.get()>
                <div class="chat-start">
                    <p class="muted">"This is the beginning of the conversation."</p>
                </div>
            </Show>
            {move || {
                let messages = ctx.messages.get();
                let mut previous: Option<&Message> = None;
                let mut rows = Vec::with_capacity(messages.len());
                for message in &messages {
                    let new_day = previous.is_none_or(|p| !same_day(p.created_at, message.created_at));
                    let grouped = !new_day
                        && previous.is_some_and(|p| {
                            p.author.id == message.author.id && message.created_at - p.created_at < GROUP_MS
                        });
                    if new_day {
                        rows.push(view! {
                            <div class="day-divider"><span>{day_label(message.created_at)}</span></div>
                        }.into_any());
                    }
                    rows.push(view! { <MessageRow message=message.clone() grouped/> }.into_any());
                    previous = Some(message);
                }
                rows
            }}
        </div>
    }
}

#[component]
fn MessageRow(message: Message, grouped: bool) -> impl IntoView {
    let ctx = use_ctx();
    let me = ctx.my_id();
    let mine = message.author.id == me;
    let owner = ctx.current_community().is_some_and(|c| c.owner_id == me);
    let can_delete = mine || (owner && message.channel_id.is_some());
    let id = message.id.clone();
    let editing = {
        let id = id.clone();
        move || ctx.editing.get().as_deref() == Some(id.as_str())
    };
    let draft = RwSignal::new(message.content.clone());
    // Put the cursor at the end of the text when the edit box opens.
    let edit_box: NodeRef<Textarea> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(el) = edit_box.get() {
            let _ = el.focus();
            let end = el.value().encode_utf16().count() as u32;
            let _ = el.set_selection_range(end, end);
        }
    });

    let delete = {
        let id = id.clone();
        move |_| {
            if !confirm("Delete this message?") {
                return;
            }
            let id = id.clone();
            spawn_local(async move {
                if let Err(error) = api::delete(&format!("/messages/{id}")).await {
                    ctx.fail(error);
                }
            });
        }
    };
    let start_edit = {
        let id = id.clone();
        let content = message.content.clone();
        move |_| {
            draft.set(content.clone());
            ctx.editing.set(Some(id.clone()));
        }
    };
    let save = {
        let id = id.clone();
        Callback::new(move |_: ()| {
            let content = match clean_message(&draft.get_untracked()) {
                Ok(content) => content,
                Err(problem) => return ctx.error(problem),
            };
            let id = id.clone();
            ctx.editing.set(None);
            spawn_local(async move {
                if let Err(error) =
                    api::patch::<_, Message>(&format!("/messages/{id}"), &ContentRequest { content }).await
                {
                    ctx.fail(error);
                }
            });
        })
    };
    let save_click = move |_| save.run(());
    let edit_keys = move |ev: KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            save.run(());
        } else if ev.key() == "Escape" {
            ctx.editing.set(None);
        }
    };

    let author = message.author.clone();
    let time = stamp(message.created_at);
    let short_time = clock(message.created_at);
    let edited = message.edited_at.map(|at| view! { <span class="edited" title=stamp(at)>"(edited)"</span> });
    let content = message.content.clone();

    view! {
        <article class="message" class:grouped=grouped class:mine=mine>
            <div class="message-gutter">
                {if grouped {
                    view! { <time class="hover-time" title=time.clone()>{short_time}</time> }.into_any()
                } else {
                    view! { <Avatar user=author.clone()/> }.into_any()
                }}
            </div>
            <div class="message-main">
                {(!grouped).then(|| view! {
                    <div class="message-head">
                        <span class="author">{author.display_name.clone()}</span>
                        <time title=time.clone()>{time.clone()}</time>
                    </div>
                })}
                {move || if editing() {
                    view! {
                        <div class="edit-box">
                            <textarea
                                node_ref=edit_box
                                class="edit-input"
                                maxlength=MAX_MESSAGE_LEN.to_string()
                                prop:value=move || draft.get()
                                on:input=move |ev| draft.set(event_target_value(&ev))
                                on:keydown=edit_keys
                            ></textarea>
                            <div class="edit-hint">
                                "Enter to save · Esc to cancel · "
                                <button class="link" on:click=save_click>"save"</button>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="message-body">{content.clone()} {edited.clone()}</div>
                    }.into_any()
                }}
            </div>
            <div class="message-actions">
                {mine.then(|| view! { <button class="icon-btn" title="Edit" on:click=start_edit.clone()>"✎"</button> })}
                {can_delete.then(|| view! { <button class="icon-btn danger" title="Delete" on:click=delete.clone()>"🗑"</button> })}
            </div>
        </article>
    }
}

#[component]
fn TypingLine() -> impl IntoView {
    let ctx = use_ctx();
    let names = move || {
        let view = ctx.view.get();
        let Some(scope) = view.scope() else { return String::new() };
        let names: Vec<String> = ctx
            .typing
            .get()
            .into_iter()
            .filter(|t| t.scope == scope)
            .map(|t| t.user.display_name)
            .collect();
        match names.as_slice() {
            [] => String::new(),
            [one] => format!("{one} is typing…"),
            [a, b] => format!("{a} and {b} are typing…"),
            _ => "Several people are typing…".to_string(),
        }
    };
    view! { <div class="typing" aria-live="polite">{names}</div> }
}

#[component]
fn Composer(placeholder: String) -> impl IntoView {
    let ctx = use_ctx();
    let draft = RwSignal::new(String::new());
    let input: NodeRef<Textarea> = NodeRef::new();
    let last_typing = StoredValue::new(0.0f64);
    let sending = RwSignal::new(false);

    // Start typing straight away when a channel opens.
    Effect::new(move |_| {
        ctx.view.track();
        if let Some(el) = input.get() {
            let _ = el.focus();
        }
    });

    let send = move || {
        let content = match clean_message(&draft.get_untracked()) {
            Ok(content) => content,
            Err(_) => return,
        };
        let path = match ctx.view.get_untracked() {
            View::Community { channel, .. } => format!("/channels/{channel}/messages"),
            View::Home { conversation: Some(id) } => format!("/dms/{id}/messages"),
            View::Home { conversation: None } => return,
        };
        draft.set(String::new());
        sending.set(true);
        spawn_local(async move {
            match api::post::<_, Message>(&path, &ContentRequest { content: content.clone() }).await {
                Ok(message) => ctx.handle(paracord_shared::ServerEvent::MessageCreated { message }),
                Err(error) => {
                    // Put the text back so nothing is lost.
                    if draft.try_get_untracked().is_some_and(|d| d.is_empty()) {
                        draft.try_set(content);
                    }
                    ctx.fail(error);
                }
            }
            sending.try_set(false);
        });
    };

    let on_input = move |ev| {
        draft.set(event_target_value(&ev));
        let now = now_ms();
        if now - last_typing.get_value() > 3000.0 && !draft.get_untracked().trim().is_empty() {
            last_typing.set_value(now);
            let event = match ctx.view.get_untracked() {
                View::Community { channel, .. } => ClientEvent::Typing {
                    channel_id: Some(channel),
                    conversation_id: None,
                },
                View::Home { conversation } => ClientEvent::Typing {
                    channel_id: None,
                    conversation_id: conversation,
                },
            };
            socket::send(&event);
        }
    };
    let on_keydown = move |ev: KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() && !ev.is_composing() {
            ev.prevent_default();
            send();
            last_typing.set_value(0.0);
        }
    };
    let rows = move || draft.with(|d| d.lines().count().clamp(1, 8)).to_string();
    let over = move || draft.with(|d| d.chars().count() > MAX_MESSAGE_LEN);

    view! {
        <div class="composer">
            <textarea
                node_ref=input
                rows=rows
                placeholder=placeholder
                aria-label="Message"
                prop:value=move || draft.get()
                on:input=on_input
                on:keydown=on_keydown
            ></textarea>
            <Show when=over>
                <span class="over-limit">{move || format!("{} / {MAX_MESSAGE_LEN}", draft.with(|d| d.chars().count()))}</span>
            </Show>
            <button
                class="send"
                title="Send"
                disabled=move || sending.get() || draft.with(|d| d.trim().is_empty()) || over()
                on:click=move |_| send()
            >
                "➤"
            </button>
        </div>
    }
}
