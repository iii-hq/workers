use iii_sdk::RegisterFunction;
use storage::handlers::{
    delete_object::{DeleteReq, DeleteResp},
    get_object::{GetReq, GetResp},
    presign_url::{PresignReq, PresignResp},
    put_object::{PutReq, PutResp},
};

mod e2e;

#[test]
fn binary_name_matches_manifest() {
    assert_eq!(storage::worker_name(), "storage");
}

/// Regression: every RPC must register through the typed
/// `RegisterFunction::new_async` so the engine receives auto-generated schemas.
#[test]
fn registered_functions_carry_request_and_response_schemas() {
    fn assert_schemas<T, F, Fut, R, E>(id: &str, f: F)
    where
        T: serde::de::DeserializeOwned + schemars::JsonSchema + Send + 'static,
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, E>> + Send + 'static,
        R: serde::Serialize + schemars::JsonSchema + Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        let reg = RegisterFunction::new_async(id, f);
        assert!(
            reg.request_format().is_some(),
            "{id} missing request_format"
        );
        assert!(
            reg.response_format().is_some(),
            "{id} missing response_format"
        );
    }

    async fn _put(_: PutReq) -> Result<PutResp, String> {
        unreachable!()
    }
    async fn _get(_: GetReq) -> Result<GetResp, String> {
        unreachable!()
    }
    async fn _del(_: DeleteReq) -> Result<DeleteResp, String> {
        unreachable!()
    }
    async fn _pre(_: PresignReq) -> Result<PresignResp, String> {
        unreachable!()
    }

    assert_schemas("storage::putObject", _put);
    assert_schemas("storage::getObject", _get);
    assert_schemas("storage::deleteObject", _del);
    assert_schemas("storage::presignUrl", _pre);
}
