//! Registration and the wire-surface catalog golden-tested in
//! `tests/schemas.rs`. Every handler parses its input without echoing it back
//! (a value or a key pasted in the wrong field must not surface in an error).

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::ctx::{Ctx, ON_CHANGE_FN};
use crate::error::{CloudError, Res};
use crate::types::*;

pub const CONNECT_ID: &str = "iii-cloud::connect";
pub const STATUS_ID: &str = "iii-cloud::status";
pub const RESOLVE_ID: &str = "iii-cloud::project::resolve";
pub const USE_ID: &str = "iii-cloud::project::use";
pub const CREATE_PROJECT_ID: &str = "iii-cloud::project::create";
pub const CREATE_ENVIRONMENT_ID: &str = "iii-cloud::project::create_environment";
pub const API_ID: &str = "iii-cloud::api";
pub const API_WRITE_ID: &str = "iii-cloud::api::write";
pub const CHECK_ID: &str = "iii-cloud::check";
pub const CHECK_GET_ID: &str = "iii-cloud::check::get";
pub const SECRET_ID: &str = "iii-cloud::secret";
pub const PROVIDE_ID: &str = "iii-cloud::secret::provide";
pub const DEPLOY_ID: &str = "iii-cloud::deploy";
pub const WATCH_ID: &str = "iii-cloud::watch";
pub const HANDOFF_ID: &str = "iii-cloud::handoff";
pub const HANDOFF_STOP_ID: &str = "iii-cloud::handoff::stop";
pub const HANDOFF_PROGRESS_ID: &str = "iii-cloud::handoff::progress";
pub const HANDOFF_BRIEF_ID: &str = "iii-cloud::handoff::brief";
pub const HANDOFF_WORKTREE_ID: &str = "iii-cloud::handoff::worktree";
pub const VALUES_ID: &str = "iii-cloud::values";
pub const VALUES_SAVE_ID: &str = "iii-cloud::values::save";
pub const VALUES_SKIP_ID: &str = "iii-cloud::values::skip";
pub const CONFIG_IMPORT_ID: &str = "iii-cloud::config::import";
pub const PROBLEM_SKIP_ID: &str = "iii-cloud::problem::skip";
pub const OVERRIDE_CHOOSE_ID: &str = "iii-cloud::override::choose";

pub const CONNECT_DESC: &str = "Sign in with the browser (the person types a code) or with an API key saved in the vault. A key connects with the secret's name alone.";
pub const STATUS_DESC: &str = "The stored state: connection, project, latest check, run, secrets, pull request and deploy. refresh re-reads the checkout, the pull request, the variables and the deployments.";
pub const RESOLVE_DESC: &str = "Read the local folder and find its project and environment in iii cloud. While a project with a workspace is selected, the folder is kept as status.candidate, not selected: project::use selects it.";
pub const USE_DESC: &str = "Select the environment and prepare a worktree for you to work in.";
pub const CREATE_PROJECT_DESC: &str = "The page's own: import the repository as a new cloud project with its first environment, on the person's explicit choice, then select it (while another project is selected, it becomes status.candidate instead). Agents never call it.";
pub const CREATE_ENVIRONMENT_DESC: &str = "The page's own: make a new environment in a cloud project (branch, compose file and copy_from default sensibly), on the person's explicit choice, then select it (an environment of status.candidate's project is not selected). Agents never call it.";
pub const API_DESC: &str = "Read any iii cloud route as the person (GET only); temporary credentials are refused. A change is PERSON_ONLY: the person makes it on the page.";
pub const API_WRITE_DESC: &str = "The page's own: change an iii cloud route as the person (POST, PUT, PATCH, DELETE); secret values are refused. Agents never call it.";
pub const CHECK_DESC: &str = "The cloud plan for a commit plus the fast checks here; with run: true, also the run here. manifest checks another worker-compose*.yaml instead of the environment's, changing nothing in iii cloud. push: true pushes the workspace's fix branch first, never an environment's branch.";
pub const CHECK_GET_DESC: &str = "Read one kept check by id (status.checks lists them, a deploy names its check_id), as it was recorded; nothing runs again.";
pub const SECRET_DESC: &str = "Ask the person for a secret value on the page, or mark a name as not needed or their decision.";
pub const PROVIDE_DESC: &str = "The page's own: the person's secret value goes to iii cloud and the worktree's .env. Agents never call it, and the value never comes back.";
pub const DEPLOY_DESC: &str = "The page's own: merge the pull request and deploy, push the checked commit to an environment's own branch and deploy it, or confirm a waiting deployment, pinned to the plan the person saw. A check of another compose file switches the environment to it first. Only the person deploys; agents never call it.";
pub const WATCH_DESC: &str = "Follow a deploy to Ready and bring back the evidence when it fails.";
pub const HANDOFF_DESC: &str = "The page's own: hand work to the agent. Returns a marker for the top of the prompt; the worker finds the agent's session by it and publishes what the agent is doing as status.agent.";
pub const HANDOFF_STOP_DESC: &str = "The page's own: stop the agent's running turn for a hand-off, with the harness's own stop. Agents never call it.";
pub const HANDOFF_PROGRESS_DESC: &str = "Tell the person which problems you start on: call it before each group of fixes with the problem ids or codes (a code stands for every open problem of it) and a short plain note. Clears itself when those problems leave a newer check or the hand-off ends.";
pub const HANDOFF_BRIEF_DESC: &str = "Your brief, and only it: where to work, the base, the compose file, the problems (id, code, owner, severity, title, detail, file, line, fix) split into yours and the person's, the deploy failure when there is one, the next step, and run_failure ({stage, code, message, log_tail}) when your hand-off's latest local run failed. handoff_id is the id or the 8 characters of the marker line.";
pub const HANDOFF_WORKTREE_DESC: &str = "Your hand-off's own worktree: {path, branch, base, created, merged_from, conflict}. Work only there, never in the person's folder. A hand-off that points at the person's folder gets one now (from the environment's branch, the person's branch merged in, their changed files brought, never their secrets) and works there from then on; the page's selection does not change. handoff_id defaults to the selected environment's open hand-off.";
pub const VALUES_DESC: &str = "The names the compose file reads (`${NAME}`, `${NAME:-}`, `${NAME:-default}` in containers) for the selected environment: whether each is set in iii cloud, secret or variable, and where its value can come from (a .env file of the person's folder, the environment's address, a copy from another environment). Names, file names and flags only: never a secret's value. manifest: another worker-compose*.yaml; from: checkout reads the person's folder.";
pub const VALUES_SAVE_DESC: &str = "The page's own: write the values the person chose to iii cloud, one by one, and the run's .env where the compose file reads `${NAME}`. Agents never call it.";
pub const VALUES_SKIP_DESC: &str = "The page's own: skip names for now in the selected environment (`skip: false` undoes it). A skipped name gets an empty value in the run here and no LOCAL_VARIABLE_UNSET; EMPTY_DEFAULT_VARIABLE turns to info; SECRET_MISSING stays a block. Agents never call it.";
pub const PROBLEM_SKIP_DESC: &str = "The page's own: skip a cloud warning for now in the selected environment (`skip: false` undoes it); only LOCAL_CONFIG_NOT_SET. The next check shows it skipped, as info. Agents never call it.";
pub const CONFIG_IMPORT_DESC: &str = "The page's own: copy the compose file's local config folder (engine.workers.configuration, fs adapter, from the person's folder) into the selected environment's console config in iii cloud, one entry per container. dry_run previews it and writes nothing; applying needs the dry run's version as expected_version. Secret-looking keys and values are left out, by name. Agents never call it.";
pub const OVERRIDE_CHOOSE_DESC: &str = "The page's own: keep the person's choice for a console override of the newest check (file: the file's value wins at deploy; console: keep the console's, which takes a change to the file). A changed console or file value drops it. Agents never call it.";
pub const ON_CHANGE_DESC: &str = "Called after each iii-cloud record changes (connection, project, check, run, secret, deploy); returns the event. Takes no args; observe it with filters on input.kind and input.terminal. Only the iii-cloud worker calls it.";

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The public wire surface, in registration order. The internal
/// `iii-cloud::on-config-change` and `iii-cloud::ui-content` are not listed.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ConnectRequest, Connection>(CONNECT_ID, CONNECT_DESC),
        spec::<StatusRequest, Status>(STATUS_ID, STATUS_DESC),
        spec::<ResolveRequest, Project>(RESOLVE_ID, RESOLVE_DESC),
        spec::<UseRequest, Project>(USE_ID, USE_DESC),
        spec::<ProjectCreateRequest, CreateResponse>(CREATE_PROJECT_ID, CREATE_PROJECT_DESC),
        spec::<EnvironmentCreateRequest, CreateResponse>(
            CREATE_ENVIRONMENT_ID,
            CREATE_ENVIRONMENT_DESC,
        ),
        spec::<ApiRequest, ApiResponse>(API_ID, API_DESC),
        spec::<ApiRequest, ApiResponse>(API_WRITE_ID, API_WRITE_DESC),
        spec::<CheckRequest, Check>(CHECK_ID, CHECK_DESC),
        spec::<CheckGetRequest, Check>(CHECK_GET_ID, CHECK_GET_DESC),
        spec::<SecretAsk, SecretRequest>(SECRET_ID, SECRET_DESC),
        spec::<ProvideRequest, SecretRequest>(PROVIDE_ID, PROVIDE_DESC),
        spec::<DeployRequest, Deploy>(DEPLOY_ID, DEPLOY_DESC),
        spec::<WatchRequest, Deploy>(WATCH_ID, WATCH_DESC),
        spec::<HandoffRequest, HandoffResponse>(HANDOFF_ID, HANDOFF_DESC),
        spec::<HandoffStopRequest, HandoffStopResponse>(HANDOFF_STOP_ID, HANDOFF_STOP_DESC),
        spec::<HandoffProgressRequest, HandoffProgressResponse>(
            HANDOFF_PROGRESS_ID,
            HANDOFF_PROGRESS_DESC,
        ),
        spec::<HandoffBriefRequest, HandoffBriefResponse>(HANDOFF_BRIEF_ID, HANDOFF_BRIEF_DESC),
        spec::<HandoffWorktreeRequest, HandoffWorktree>(HANDOFF_WORKTREE_ID, HANDOFF_WORKTREE_DESC),
        spec::<ValuesRequest, Values>(VALUES_ID, VALUES_DESC),
        spec::<ValuesSave, ValuesSaved>(VALUES_SAVE_ID, VALUES_SAVE_DESC),
        spec::<ValuesSkip, ValuesSkipped>(VALUES_SKIP_ID, VALUES_SKIP_DESC),
        spec::<ProblemSkip, ProblemsSkipped>(PROBLEM_SKIP_ID, PROBLEM_SKIP_DESC),
        spec::<ConfigImport, ConfigImported>(CONFIG_IMPORT_ID, CONFIG_IMPORT_DESC),
        spec::<OverrideChoose, OverrideChosen>(OVERRIDE_CHOOSE_ID, OVERRIDE_CHOOSE_DESC),
        spec::<ChangeEvent, ChangeEvent>(ON_CHANGE_FN, ON_CHANGE_DESC),
    ]
}

/// Deserialize without quoting the input back: only a missing field is named.
pub fn parse_request<T: DeserializeOwned>(payload: Value) -> Res<T> {
    let payload = if payload.is_null() {
        json!({})
    } else {
        payload
    };
    serde_json::from_value(payload).map_err(|error| {
        let text = error.to_string();
        let missing = text
            .strip_prefix("missing field `")
            .and_then(|rest| rest.split('`').next())
            .filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        CloudError::invalid(match missing {
            Some(field) => format!("missing field `{field}`"),
            None => "the request does not match the function's input".to_string(),
        })
    })
}

fn register<Req, Resp, F, Fut>(
    iii: &IIIClient,
    ctx: &Arc<Ctx>,
    id: &'static str,
    desc: &'static str,
    hidden: bool,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Res<Resp>> + Send + 'static,
{
    let ctx = ctx.clone();
    let handler = Arc::new(handler);
    let mut registration = RegisterFunction::new_async(move |payload: Value| {
        let ctx = ctx.clone();
        let handler = handler.clone();
        async move {
            let request = parse_request::<Req>(payload)?;
            match handler(ctx, request).await {
                Ok(response) => Ok::<Resp, Error>(response),
                Err(error) => {
                    tracing::debug!(function = id, code = %error.code, "request refused");
                    Err(error.into())
                }
            }
        }
    })
    .description(desc)
    .request_format(serde_json::to_value(schema_of::<Req>()).expect("schema serializes"))
    .response_format(serde_json::to_value(schema_of::<Resp>()).expect("schema serializes"));
    if hidden {
        // An observer would get a copy of the input and output (a code, a
        // value); spans of these calls stay out of the default trace view.
        registration = registration
            .observable(false)
            .metadata(json!({ "trace_hidden": true }));
    }
    iii.register_function(id, registration);
}

/// Register every `iii-cloud::*` function. Keep in lockstep with [`catalog`].
pub fn register_all(iii: &IIIClient, ctx: &Arc<Ctx>) {
    iii.register_function(
        ON_CHANGE_FN,
        RegisterFunction::new(|event: ChangeEvent| Ok::<_, Error>(event))
            .description(ON_CHANGE_DESC)
            .private(),
    );
    register(
        iii,
        ctx,
        CONNECT_ID,
        CONNECT_DESC,
        true,
        |c, r: ConnectRequest| async move { crate::connect::connect(&c, r).await },
    );
    register(
        iii,
        ctx,
        STATUS_ID,
        STATUS_DESC,
        false,
        |c, r: StatusRequest| async move { crate::status::status(&c, r).await },
    );
    register(
        iii,
        ctx,
        RESOLVE_ID,
        RESOLVE_DESC,
        false,
        |c, r: ResolveRequest| async move { crate::project::resolve(&c, r).await },
    );
    register(
        iii,
        ctx,
        USE_ID,
        USE_DESC,
        false,
        |c, r: UseRequest| async move { crate::project::use_project(&c, r).await },
    );
    register(
        iii,
        ctx,
        CREATE_PROJECT_ID,
        CREATE_PROJECT_DESC,
        false,
        |c, r: ProjectCreateRequest| async move { crate::create::create_project(&c, r).await },
    );
    register(
        iii,
        ctx,
        CREATE_ENVIRONMENT_ID,
        CREATE_ENVIRONMENT_DESC,
        false,
        |c, r: EnvironmentCreateRequest| async move { crate::create::create_environment(&c, r).await },
    );
    register(
        iii,
        ctx,
        API_ID,
        API_DESC,
        false,
        |c, r: ApiRequest| async move { crate::api::api(&c, r).await },
    );
    register(
        iii,
        ctx,
        API_WRITE_ID,
        API_WRITE_DESC,
        false,
        |c, r: ApiRequest| async move { crate::api::write(&c, r).await },
    );
    register(
        iii,
        ctx,
        CHECK_ID,
        CHECK_DESC,
        false,
        |c, r: CheckRequest| async move { crate::check::check(&c, r).await },
    );
    register(
        iii,
        ctx,
        CHECK_GET_ID,
        CHECK_GET_DESC,
        false,
        |c, r: CheckGetRequest| async move { crate::check::get(&c, r) },
    );
    register(
        iii,
        ctx,
        SECRET_ID,
        SECRET_DESC,
        false,
        |c, r: SecretAsk| async move { crate::secret::ask(&c, r).await },
    );
    register(
        iii,
        ctx,
        PROVIDE_ID,
        PROVIDE_DESC,
        true,
        |c, r: ProvideRequest| async move { crate::secret::provide(&c, r).await },
    );
    register(
        iii,
        ctx,
        DEPLOY_ID,
        DEPLOY_DESC,
        false,
        |c, r: DeployRequest| async move { crate::deploy::deploy(&c, r).await },
    );
    register(
        iii,
        ctx,
        HANDOFF_ID,
        HANDOFF_DESC,
        false,
        |c, r: HandoffRequest| async move { crate::handoff::create(&c, r).await },
    );
    register(
        iii,
        ctx,
        HANDOFF_STOP_ID,
        HANDOFF_STOP_DESC,
        false,
        |c, r: HandoffStopRequest| async move { crate::handoff::stop(&c, r).await },
    );
    register(
        iii,
        ctx,
        HANDOFF_PROGRESS_ID,
        HANDOFF_PROGRESS_DESC,
        false,
        |c, r: HandoffProgressRequest| async move { crate::handoff::progress(&c, r).await },
    );
    register(
        iii,
        ctx,
        HANDOFF_BRIEF_ID,
        HANDOFF_BRIEF_DESC,
        false,
        |c, r: HandoffBriefRequest| async move { crate::handoff::brief_for(&c, r).await },
    );
    register(
        iii,
        ctx,
        HANDOFF_WORKTREE_ID,
        HANDOFF_WORKTREE_DESC,
        false,
        |c, r: HandoffWorktreeRequest| async move { crate::handoff::worktree(&c, r).await },
    );
    register(
        iii,
        ctx,
        VALUES_ID,
        VALUES_DESC,
        false,
        |c, r: ValuesRequest| async move { crate::values::values(&c, r).await },
    );
    // Its input holds values: hidden from traces and observers.
    register(
        iii,
        ctx,
        VALUES_SAVE_ID,
        VALUES_SAVE_DESC,
        true,
        |c, r: ValuesSave| async move { crate::values::save(&c, r).await },
    );
    register(
        iii,
        ctx,
        VALUES_SKIP_ID,
        VALUES_SKIP_DESC,
        false,
        |c, r: ValuesSkip| async move { crate::values::skip(&c, r).await },
    );
    register(
        iii,
        ctx,
        PROBLEM_SKIP_ID,
        PROBLEM_SKIP_DESC,
        false,
        |c, r: ProblemSkip| async move { crate::values::skip_problem(&c, r).await },
    );
    // Its preview returns config values: hidden from traces and observers.
    register(
        iii,
        ctx,
        CONFIG_IMPORT_ID,
        CONFIG_IMPORT_DESC,
        true,
        |c, r: ConfigImport| async move { crate::console::import(&c, r).await },
    );
    register(
        iii,
        ctx,
        OVERRIDE_CHOOSE_ID,
        OVERRIDE_CHOOSE_DESC,
        false,
        |c, r: OverrideChoose| async move { crate::console::choose(&c, r).await },
    );
    register(
        iii,
        ctx,
        WATCH_ID,
        WATCH_DESC,
        false,
        |c, r: WatchRequest| async move { crate::watch::watch(&c, r).await },
    );
}
