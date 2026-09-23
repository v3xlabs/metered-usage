pub mod api;
pub mod assets;
pub mod auth;
pub mod mcp;

use std::sync::Arc;

use poem::{Endpoint, EndpointExt, IntoEndpoint, Route};
use poem_openapi::OpenApiService;

use crate::app::AppState;
use crate::http::api::{
    account, analytics, dead_letter, health, leverage, plan, price, quota, session, source, usage,
};
use crate::http::auth::RequireCredential;

const TITLE: &str = "metered-usage";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MOUNT: &str = "/api";

type Apis = (
    health::HealthApi,
    session::SessionApi,
    source::SourceApi,
    account::AccountApi,
    usage::UsageApi,
    analytics::AnalyticsApi,
    dead_letter::DeadLetterApi,
    price::PriceApi,
    plan::PlanApi,
    leverage::LeverageApi,
    quota::QuotaApi,
);

pub fn service(state: &Arc<AppState>) -> OpenApiService<Apis, ()> {
    OpenApiService::new(
        (
            health::HealthApi {
                state: Arc::clone(state),
            },
            session::SessionApi {
                state: Arc::clone(state),
            },
            source::SourceApi {
                state: Arc::clone(state),
            },
            account::AccountApi {
                state: Arc::clone(state),
            },
            usage::UsageApi {
                state: Arc::clone(state),
            },
            analytics::AnalyticsApi {
                state: Arc::clone(state),
            },
            dead_letter::DeadLetterApi {
                state: Arc::clone(state),
            },
            price::PriceApi {
                state: Arc::clone(state),
            },
            plan::PlanApi {
                state: Arc::clone(state),
            },
            leverage::LeverageApi {
                state: Arc::clone(state),
            },
            quota::QuotaApi::new(Arc::clone(state)),
        ),
        TITLE,
        VERSION,
    )
    .server(MOUNT)
}

/// The document the web client is generated from.
pub fn specification(state: &Arc<AppState>) -> String {
    service(state).spec()
}

pub fn routes(state: &Arc<AppState>) -> impl Endpoint<Output = poem::Response> + use<> {
    let api = service(state);
    let document = api.spec_endpoint();

    Route::new()
        .nest(
            MOUNT,
            api.with(RequireCredential {
                token: state.token.clone(),
            }),
        )
        .at(
            "/mcp",
            mcp::endpoint(Arc::clone(state))
                .into_endpoint()
                .with(RequireCredential {
                    token: state.token.clone(),
                }),
        )
        .at("/openapi.json", document)
        .at("/*path", poem::get(assets::serve))
        .map_to_response()
}
