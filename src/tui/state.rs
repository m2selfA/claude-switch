use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Mode {
    FirstRun,
    Normal,
    Search,
    Help,
    ConfirmDelete,
    AddFullName,
    AddFullAlias,
    Message(String, bool),
    LiteProviderSelect,
    LiteKeySelect {
        provider_id: String,
    },
    LiteFetching,
    ProviderAnthropicTest {
        provider_id: String,
        key_id: String,
        source: ProviderTestSource,
        field: usize,
    },
    ProviderAnthropicOutcome {
        provider_id: String,
        key_id: String,
        source: ProviderTestSource,
        field: usize,
        model: String,
        endpoint_used: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        body: String,
        is_error: bool,
    },
    LiteModelSelect {
        profile_name: String,
        token: String,
        base_url: String,
        models: Vec<String>,
    },
    LiteEdit {
        profile_id: String,
    },
    EditProfile {
        profile_id: String,
        step: usize,
    },
    ProviderList,
    ProviderAdd {
        step: usize,
    },
    ProviderSmartPaste,
    ProviderEdit {
        provider_id: String,
        step: usize,
    },
    ProviderEditKeyInput {
        provider_id: String,
        step: usize,
    },
    ProviderKeyList {
        provider_id: String,
    },
    ProviderTestKeyList {
        provider_id: String,
    },
    ProviderKeyAdd {
        provider_id: String,
        step: usize,
    },
    ProviderKeyEdit {
        provider_id: String,
        key_id: String,
        step: usize,
        source: KeyEditSource,
    },
    ProviderKeyRename {
        provider_id: String,
        key_id: String,
        source: KeyEditSource,
    },
    ConfirmDeleteProvider {
        provider_id: String,
        name: String,
    },
    ConfirmDeleteKey {
        provider_id: String,
        key_id: String,
        name: String,
    },
    ProviderKeyInUse {
        provider_id: String,
        key_id: String,
        name: String,
        return_mode: Box<Mode>,
    },
    McpAdd {
        step: usize,
    },
    McpEdit {
        mcp_id: String,
        step: usize,
    },
    McpProfilePicker {
        profile_id: String,
    },
    McpSmartPaste,
    ConfirmDeleteMcp {
        mcp_id: String,
        name: String,
    },
    PublicSitePrompt,
    PublicSiteTesting,
    PublicSiteResults,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Page {
    Profile,
    Provider,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum KeyEditSource {
    ProviderKeyList,
    ProviderEdit,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ModelFetchState {
    Loaded,
    Empty,
    Unavailable(String),
}

pub(super) const MCP_TYPES: &[&str] = &["stdio", "http", "streamable-http", "sse"];
pub(super) const MCP_EDITOR_STEPS: usize = 13;

pub struct App {
    pub(super) manager: ProfileManager,
    pub(super) profiles: Vec<Profile>,
    pub(super) list_state: ListState,
    pub(super) list_scroll: ScrollbarState,
    pub(super) mode: Mode,
    pub(super) page: Page,
    pub(super) input_buffer: String,
    pub(super) cursor_pos: usize,
    pub(super) search_query: String,
    pub(super) filtered_indices: Vec<usize>,
    pub(super) lite_1m: [bool; 5],
    pub(super) lite_step: usize,
    pub(super) lite_models: Vec<String>,
    pub(super) lite_model_fetch_state: ModelFetchState,
    pub(super) lite_model_page: usize,
    pub(super) lite_edit_id: String,
    pub(super) lite_name: String,
    pub(super) lite_alias: String,
    pub(super) lite_token: String,
    pub(super) lite_url: String,
    pub(super) lite_provider_id: Option<String>,
    pub(super) lite_key_id: Option<String>,
    pub(super) lite_provider_keys: Vec<ProviderKey>,
    pub(super) provider_list_state: ListState,
    pub(super) provider_list_scroll: ScrollbarState,
    pub(super) provider_name_buf: String,
    pub(super) provider_url_buf: String,
    pub(super) provider_key_buf: String,
    pub(super) provider_key_name_buf: String,
    pub(super) provider_add_existing_id: Option<String>,
    pub(super) provider_smart_paste_buf: String,
    pub(super) provider_smart_paste_error: Option<String>,
    pub(super) provider_test_prompt_buf: String,
    pub(super) provider_test_model_buf: String,
    pub(super) provider_test_models: Vec<String>,
    pub(super) provider_test_model_fetch_state: ModelFetchState,
    pub(super) provider_test_model_selected: usize,
    pub(super) message_return_mode: Option<Mode>,
    pub(super) providers_cache: Vec<Provider>,
    pub(super) provider_keys_cache: Vec<ProviderKey>,
    pub(super) provider_key_selected: usize,
    pub(super) provider_key_linked_profiles: Vec<Profile>,
    pub(super) provider_key_linked_profile_selected: usize,
    pub(super) mcps_cache: Vec<McpServer>,
    pub(super) mcp_list_state: ListState,
    pub(super) mcp_list_scroll: ScrollbarState,
    pub(super) mcp_profile_links_cache: Vec<Profile>,
    pub(super) mcp_selected_ids: Vec<String>,
    pub(super) mcp_filter_buf: String,
    pub(super) mcp_name_buf: String,
    pub(super) mcp_type_idx: usize,
    pub(super) mcp_command_buf: String,
    pub(super) mcp_args: Vec<String>,
    pub(super) mcp_env: Vec<String>,
    pub(super) mcp_cwd_buf: String,
    pub(super) mcp_url_buf: String,
    pub(super) mcp_headers: Vec<String>,
    pub(super) mcp_oauth_buf: String,
    pub(super) mcp_headers_helper_buf: String,
    pub(super) mcp_timeout_buf: String,
    pub(super) mcp_always_load: Option<bool>,
    pub(super) mcp_disabled: Option<bool>,
    pub(super) public_site_prompt_buf: String,
    pub(super) public_site_targets: Vec<PublicSiteTarget>,
    pub(super) public_site_results: Vec<PublicSiteTestResult>,
    pub(super) public_site_result_selected: usize,
    pub(super) public_site_detail_scroll: u16,
    pub(super) public_site_completed: usize,
    pub(super) public_site_total: usize,
    pub(super) public_site_status: String,
    pub(super) public_site_event_rx: Option<mpsc::Receiver<PublicSiteWorkerEvent>>,
    pub(super) lite_mod_opus: String,
    pub(super) lite_mod_sonnet: String,
    pub(super) lite_mod_haiku: String,
    pub(super) lite_mod_model: String,
    pub(super) lite_mod_subagent: String,
    pub(super) lite_extras: Vec<String>,
    pub(super) lite_launch_args: String,
}

impl App {
    pub fn new(manager: ProfileManager) -> Result<Self> {
        let profiles = manager.list_profiles()?;
        let filtered_indices: Vec<usize> = (0..profiles.len()).collect();
        let mut list_state = ListState::default();
        if !profiles.is_empty() {
            list_state.select(Some(0));
        }

        let (mode, input_buffer) = if profiles.is_empty() {
            (Mode::FirstRun, "default".to_string())
        } else {
            (Mode::Normal, String::new())
        };

        Ok(Self {
            manager,
            profiles,
            list_state,
            list_scroll: ScrollbarState::default(),
            mode,
            page: Page::Profile,
            input_buffer,
            cursor_pos: 0,
            search_query: String::new(),
            filtered_indices,
            lite_1m: [false; 5],
            lite_step: 0,
            lite_models: Vec::new(),
            lite_model_fetch_state: ModelFetchState::Loaded,
            lite_model_page: 0,
            lite_name: String::new(),
            lite_alias: String::new(),
            lite_token: String::new(),
            lite_url: "https://api.anthropic.com".to_string(),
            lite_provider_id: None,
            lite_key_id: None,
            lite_provider_keys: Vec::new(),
            provider_list_state: ListState::default(),
            provider_list_scroll: ScrollbarState::default(),
            provider_name_buf: String::new(),
            provider_url_buf: String::new(),
            provider_key_buf: String::new(),
            provider_key_name_buf: String::new(),
            provider_add_existing_id: None,
            provider_smart_paste_buf: String::new(),
            provider_smart_paste_error: None,
            provider_test_prompt_buf: "Hello".to_string(),
            provider_test_model_buf: String::new(),
            provider_test_models: Vec::new(),
            provider_test_model_fetch_state: ModelFetchState::Loaded,
            provider_test_model_selected: 0,
            message_return_mode: None,
            providers_cache: Vec::new(),
            provider_keys_cache: Vec::new(),
            provider_key_selected: 0,
            provider_key_linked_profiles: Vec::new(),
            provider_key_linked_profile_selected: 0,
            mcps_cache: Vec::new(),
            mcp_list_state: ListState::default(),
            mcp_list_scroll: ScrollbarState::default(),
            mcp_profile_links_cache: Vec::new(),
            mcp_selected_ids: Vec::new(),
            mcp_filter_buf: String::new(),
            mcp_name_buf: String::new(),
            mcp_type_idx: 0,
            mcp_command_buf: String::new(),
            mcp_args: Vec::new(),
            mcp_env: Vec::new(),
            mcp_cwd_buf: String::new(),
            mcp_url_buf: String::new(),
            mcp_headers: Vec::new(),
            mcp_oauth_buf: String::new(),
            mcp_headers_helper_buf: String::new(),
            mcp_timeout_buf: String::new(),
            mcp_always_load: None,
            mcp_disabled: None,
            public_site_prompt_buf: PUBLIC_SITE_TEST_DEFAULT_PROMPT.to_string(),
            public_site_targets: Vec::new(),
            public_site_results: Vec::new(),
            public_site_result_selected: 0,
            public_site_detail_scroll: 0,
            public_site_completed: 0,
            public_site_total: 0,
            public_site_status: String::new(),
            public_site_event_rx: None,
            lite_edit_id: String::new(),
            lite_mod_opus: String::new(),
            lite_mod_sonnet: String::new(),
            lite_mod_haiku: String::new(),
            lite_mod_model: String::new(),
            lite_mod_subagent: String::new(),
            lite_extras: Vec::new(),
            lite_launch_args: "--dangerously-skip-permissions".to_string(),
        })
    }
}
