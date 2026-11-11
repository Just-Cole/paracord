//! App-wide state. Everything lives in signals inside one `Ctx`, which every
//! component reads with `use_ctx()`.

use std::collections::HashSet;
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;
use paracord_shared::{Community, Conversation, Member, Message, ServerEvent, User};

use crate::{api, socket, util};

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home { conversation: Option<String> },
    Community { community: String, channel: String },
}

impl View {
    /// The channel or conversation whose messages are on screen.
    pub fn scope(&self) -> Option<&str> {
        match self {
            View::Home { conversation } => conversation.as_deref(),
            View::Community { channel, .. } => Some(channel),
        }
    }

    pub fn community(&self) -> Option<&str> {
        match self {
            View::Community { community, .. } => Some(community),
            View::Home { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    CreateCommunity,
    Discover,
    NewChannel(String),
    EditChannel(paracord_shared::Channel),
    RenameCommunity(Community),
    NewDm,
    Profile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Typist {
    pub user: User,
    pub scope: String,
    pub until: f64,
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub me: RwSignal<Option<User>>,
    /// False until the first "am I signed in?" check finishes.
    pub ready: RwSignal<bool>,
    pub communities: RwSignal<Vec<Community>>,
    pub dms: RwSignal<Vec<Conversation>>,
    pub view: RwSignal<View>,
    pub messages: RwSignal<Vec<Message>>,
    pub has_more: RwSignal<bool>,
    pub loading: RwSignal<bool>,
    pub members: RwSignal<Vec<Member>>,
    pub typing: RwSignal<Vec<Typist>>,
    pub unread: RwSignal<HashSet<String>>,
    pub modal: RwSignal<Option<Modal>>,
    pub toast: RwSignal<Option<String>>,
    pub connected: RwSignal<bool>,
    pub editing: RwSignal<Option<String>>,
}

pub fn use_ctx() -> Ctx {
    expect_context::<Ctx>()
}

impl Ctx {
    pub fn new() -> Self {
        Self {
            me: RwSignal::new(None),
            ready: RwSignal::new(false),
            communities: RwSignal::new(Vec::new()),
            dms: RwSignal::new(Vec::new()),
            view: RwSignal::new(View::Home { conversation: None }),
            messages: RwSignal::new(Vec::new()),
            has_more: RwSignal::new(false),
            loading: RwSignal::new(false),
            members: RwSignal::new(Vec::new()),
            typing: RwSignal::new(Vec::new()),
            unread: RwSignal::new(HashSet::new()),
            modal: RwSignal::new(None),
            toast: RwSignal::new(None),
            connected: RwSignal::new(false),
            editing: RwSignal::new(None),
        }
    }

    pub fn my_id(&self) -> String {
        self.me
            .get_untracked()
            .map(|u| u.id)
            .unwrap_or_default()
    }

    pub fn current_community(&self) -> Option<Community> {
        let view = self.view.get();
        let id = view.community()?;
        self.communities.get().into_iter().find(|c| c.id == id)
    }

    pub fn error(&self, message: impl Into<String>) {
        let message = message.into();
        self.toast.set(Some(message.clone()));
        let toast = self.toast;
        set_timeout(
            move || {
                if toast.get_untracked().as_deref() == Some(message.as_str()) {
                    toast.set(None);
                }
            },
            Duration::from_secs(5),
        );
    }

    /// Reports a failed request. A 401 means the session ended: sign out.
    pub fn fail(&self, error: api::ApiError) {
        if error.unauthorized() {
            self.signed_out();
        } else {
            self.error(error.message);
        }
    }

    // ---------- Sign in / out ----------

    pub async fn check_session(self) {
        match api::get::<User>("/me").await {
            Ok(user) => self.signed_in(user),
            Err(error) if !error.unauthorized() => self.error(error.message),
            Err(_) => {}
        }
        self.ready.set(true);
    }

    pub fn signed_in(self, user: User) {
        self.me.set(Some(user));
        socket::connect(self);
        spawn_local(async move {
            self.refresh_communities().await;
            self.refresh_dms().await;
        });
    }

    pub fn signed_out(self) {
        socket::disconnect();
        self.me.set(None);
        self.communities.set(Vec::new());
        self.dms.set(Vec::new());
        self.messages.set(Vec::new());
        self.members.set(Vec::new());
        self.unread.set(HashSet::new());
        self.modal.set(None);
        self.view.set(View::Home { conversation: None });
    }

    pub fn sign_out(self) {
        spawn_local(async move {
            let _ = api::post_empty("/auth/logout").await;
            self.signed_out();
        });
    }

    // ---------- Loading ----------

    pub async fn refresh_communities(self) {
        match api::get::<Vec<Community>>("/communities").await {
            Ok(list) => {
                self.communities.set(list);
                self.fix_view();
            }
            Err(error) => self.fail(error),
        }
    }

    pub async fn refresh_dms(self) {
        match api::get::<Vec<Conversation>>("/dms").await {
            Ok(list) => self.dms.set(list),
            Err(error) => self.fail(error),
        }
    }

    /// Keeps the view pointing at something that still exists.
    fn fix_view(self) {
        let communities = self.communities.get_untracked();
        let first = || {
            communities
                .iter()
                .find_map(|c| c.channels.first().map(|ch| (c.id.clone(), ch.id.clone())))
        };
        match self.view.get_untracked() {
            View::Community { community, channel } => {
                match communities.iter().find(|c| c.id == community) {
                    Some(c) if c.channels.iter().any(|ch| ch.id == channel) => {}
                    Some(c) => {
                        if let Some(ch) = c.channels.first() {
                            self.open(View::Community {
                                community: c.id.clone(),
                                channel: ch.id.clone(),
                            });
                        }
                    }
                    None => match first() {
                        Some((community, channel)) => self.open(View::Community { community, channel }),
                        None => self.open(View::Home { conversation: None }),
                    },
                }
            }
            View::Home { conversation: None } => {
                // First load lands in the first community instead of an empty home.
                if self.messages.get_untracked().is_empty()
                    && let Some((community, channel)) = first()
                {
                    self.open(View::Community { community, channel });
                }
            }
            View::Home { .. } => {}
        }
    }

    pub fn open_community(self, id: &str) {
        let target = self
            .communities
            .get_untracked()
            .into_iter()
            .find(|c| c.id == id)
            .and_then(|c| c.channels.first().map(|ch| (c.id.clone(), ch.id.clone())));
        if let Some((community, channel)) = target {
            self.open(View::Community { community, channel });
        }
    }

    pub fn open(self, view: View) {
        let old = self.view.get_untracked();
        if old == view {
            return;
        }
        let community_changed = old.community() != view.community();
        if let Some(scope) = view.scope() {
            let scope = scope.to_string();
            self.unread.update(|set| {
                set.remove(&scope);
            });
        }
        self.editing.set(None);
        self.messages.set(Vec::new());
        self.has_more.set(false);
        self.view.set(view);
        spawn_local(async move { self.load_messages(false).await });
        if community_changed {
            self.members.set(Vec::new());
            spawn_local(async move { self.load_members().await });
        }
    }

    fn messages_path(view: &View) -> Option<String> {
        match view {
            View::Community { channel, .. } => Some(format!("/channels/{channel}/messages")),
            View::Home { conversation: Some(id) } => Some(format!("/dms/{id}/messages")),
            View::Home { conversation: None } => None,
        }
    }

    pub async fn load_messages(self, older: bool) {
        let view = self.view.get_untracked();
        let Some(mut path) = Self::messages_path(&view) else {
            return;
        };
        if older {
            match self.messages.get_untracked().first() {
                Some(first) => path.push_str(&format!("?before={}", first.id)),
                None => return,
            }
        }
        self.loading.set(true);
        let result = api::get::<Vec<Message>>(&path).await;
        self.loading.set(false);
        // The user may have moved on while this was loading.
        if self.view.get_untracked() != view {
            return;
        }
        match result {
            Ok(page) => {
                self.has_more.set(page.len() >= paracord_shared::MESSAGE_PAGE);
                if older {
                    self.messages.update(|list| {
                        let mut page = page;
                        page.append(list);
                        *list = page;
                    });
                } else {
                    self.messages.set(page);
                }
            }
            Err(error) => self.fail(error),
        }
    }

    pub async fn load_members(self) {
        let Some(id) = self.view.get_untracked().community().map(str::to_string) else {
            return;
        };
        match api::get::<Vec<Member>>(&format!("/communities/{id}/members")).await {
            Ok(list) => {
                if self.view.get_untracked().community() == Some(id.as_str()) {
                    self.members.set(list);
                }
            }
            Err(error) => self.fail(error),
        }
    }

    /// After the socket reconnects, catch up on anything missed.
    pub fn resync(self) {
        spawn_local(async move {
            self.refresh_communities().await;
            self.refresh_dms().await;
            self.load_messages(false).await;
            self.load_members().await;
        });
    }

    // ---------- Real-time events ----------

    pub fn handle(self, event: ServerEvent) {
        match event {
            ServerEvent::Hello { .. } | ServerEvent::Pong => {}
            ServerEvent::MessageCreated { message } => self.on_message(message),
            ServerEvent::MessageUpdated { message } => {
                self.messages.update(|list| {
                    if let Some(slot) = list.iter_mut().find(|m| m.id == message.id) {
                        *slot = message;
                    }
                });
            }
            ServerEvent::MessageDeleted { id, .. } => {
                self.messages.update(|list| list.retain(|m| m.id != id));
            }
            ServerEvent::CommunitiesChanged => {
                spawn_local(async move { self.refresh_communities().await });
            }
            ServerEvent::MembersChanged { community_id } => {
                if self.view.get_untracked().community() == Some(community_id.as_str()) {
                    spawn_local(async move { self.load_members().await });
                }
            }
            ServerEvent::DmsChanged => spawn_local(async move { self.refresh_dms().await }),
            ServerEvent::Presence { user_id, online } => {
                self.members.update(|list| {
                    if let Some(m) = list.iter_mut().find(|m| m.user.id == user_id) {
                        m.online = online;
                    }
                    list.sort_by(|a, b| {
                        b.online.cmp(&a.online).then_with(|| {
                            a.user
                                .display_name
                                .to_lowercase()
                                .cmp(&b.user.display_name.to_lowercase())
                        })
                    });
                });
            }
            ServerEvent::UserUpdated { user } => self.on_user(user),
            ServerEvent::Typing {
                user,
                channel_id,
                conversation_id,
            } => {
                let Some(scope) = channel_id.or(conversation_id) else {
                    return;
                };
                self.typing.update(|list| {
                    list.retain(|t| !(t.user.id == user.id && t.scope == scope));
                    list.push(Typist {
                        user,
                        scope,
                        until: util::now_ms() + 6000.0,
                    });
                });
            }
        }
    }

    fn on_message(self, message: Message) {
        let scope = message
            .channel_id
            .clone()
            .or_else(|| message.conversation_id.clone())
            .unwrap_or_default();
        let author = message.author.id.clone();
        // Whoever just spoke has stopped typing.
        self.typing
            .update(|list| list.retain(|t| !(t.user.id == author && t.scope == scope)));

        if let Some(conversation) = &message.conversation_id {
            let known = self.dms.get_untracked().iter().any(|c| &c.id == conversation);
            if known {
                let at = message.created_at;
                self.dms.update(|list| {
                    if let Some(c) = list.iter_mut().find(|c| &c.id == conversation) {
                        c.last_message_at = at;
                    }
                    list.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
                });
            } else {
                spawn_local(async move { self.refresh_dms().await });
            }
        }

        if self.view.get_untracked().scope() == Some(scope.as_str()) {
            self.messages.update(|list| {
                if !list.iter().any(|m| m.id == message.id) {
                    list.push(message);
                }
            });
        } else if author != self.my_id() {
            self.unread.update(|set| {
                set.insert(scope);
            });
        }
    }

    fn on_user(self, user: User) {
        if self.me.get_untracked().is_some_and(|me| me.id == user.id) {
            self.me.set(Some(user.clone()));
        }
        self.members.update(|list| {
            for m in list.iter_mut().filter(|m| m.user.id == user.id) {
                m.user = user.clone();
            }
        });
        self.messages.update(|list| {
            for m in list.iter_mut().filter(|m| m.author.id == user.id) {
                m.author = user.clone();
            }
        });
        self.dms.update(|list| {
            for c in list.iter_mut().filter(|c| c.user.id == user.id) {
                c.user = user.clone();
            }
        });
    }
}
