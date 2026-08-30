use a3s_runtime::contract::{
    RuntimeActionRequest, RuntimeApplyRequest, RuntimeCapabilities, RuntimeExecRequest,
    RuntimeExecResult, RuntimeInspection, RuntimeLogChunk, RuntimeLogQuery, RuntimeObservation,
    RuntimeRemoval, RuntimeUnitSpec,
};
use a3s_runtime::{RuntimeRequestReceipt, RuntimeUnitRecord};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

fn adjacent_schema(schema: &str, offset: i64) -> String {
    let (prefix, version) = schema
        .rsplit_once(".v")
        .expect("versioned schema must end in .v<number>");
    let version = version
        .parse::<i64>()
        .expect("schema version must be numeric");
    format!("{prefix}.v{}", version + offset)
}

fn object(value: &mut Value) -> &mut Map<String, Value> {
    value
        .as_object_mut()
        .expect("every top-level Runtime wire fixture must be an object")
}

fn assert_top_level_fixture<T>(
    raw: &str,
    expected_schema: &str,
    validate: impl Fn(&T) -> Result<(), String>,
) where
    T: DeserializeOwned + Serialize,
{
    let value: Value = serde_json::from_str(raw).expect("golden fixture must be valid JSON");
    assert_eq!(
        value.get("schema"),
        Some(&Value::String(expected_schema.into()))
    );

    let decoded: T =
        serde_json::from_value(value.clone()).expect("current golden fixture must decode");
    validate(&decoded).expect("current golden fixture must validate");
    assert_eq!(
        serde_json::to_value(&decoded).expect("golden value must encode"),
        value,
        "golden encode must preserve the complete public wire shape"
    );

    let mut missing = value.clone();
    object(&mut missing).remove("schema");
    assert!(
        serde_json::from_value::<T>(missing).is_err(),
        "a missing schema must fail closed"
    );

    for incompatible_schema in [
        adjacent_schema(expected_schema, -1),
        adjacent_schema(expected_schema, 1),
    ] {
        let mut incompatible = value.clone();
        object(&mut incompatible).insert("schema".into(), incompatible_schema.into());
        let decoded: T = serde_json::from_value(incompatible)
            .expect("a syntactically valid schema string must decode before validation");
        assert!(
            validate(&decoded).is_err(),
            "old and future schemas must fail validation"
        );
    }

    let mut malformed = value.clone();
    object(&mut malformed).insert("schema".into(), Value::Bool(true));
    assert!(
        serde_json::from_value::<T>(malformed).is_err(),
        "a non-string schema must fail decoding"
    );

    let mut unknown = value;
    object(&mut unknown).insert("unexpected".into(), Value::Bool(true));
    assert!(
        serde_json::from_value::<T>(unknown).is_err(),
        "unknown top-level fields must fail closed"
    );
}

#[test]
fn ct_schema_001_every_top_level_wire_record_has_a_versioned_golden_fixture() {
    assert_top_level_fixture::<RuntimeCapabilities>(
        include_str!("golden/capabilities-v5.json"),
        RuntimeCapabilities::SCHEMA,
        RuntimeCapabilities::validate,
    );
    assert_top_level_fixture::<RuntimeUnitSpec>(
        include_str!("golden/unit-spec-v3.json"),
        RuntimeUnitSpec::SCHEMA,
        RuntimeUnitSpec::validate,
    );
    assert_top_level_fixture::<RuntimeApplyRequest>(
        include_str!("golden/apply-request-v1.json"),
        RuntimeApplyRequest::SCHEMA,
        RuntimeApplyRequest::validate,
    );
    assert_top_level_fixture::<RuntimeActionRequest>(
        include_str!("golden/action-request-v1.json"),
        RuntimeActionRequest::SCHEMA,
        RuntimeActionRequest::validate,
    );
    assert_top_level_fixture::<RuntimeObservation>(
        include_str!("golden/observation-v3.json"),
        RuntimeObservation::SCHEMA,
        RuntimeObservation::validate,
    );
    for inspection in [
        include_str!("golden/inspection-found-v1.json"),
        include_str!("golden/inspection-not-found-v1.json"),
    ] {
        assert_top_level_fixture::<RuntimeInspection>(
            inspection,
            RuntimeInspection::SCHEMA,
            RuntimeInspection::validate,
        );
    }
    assert_top_level_fixture::<RuntimeRemoval>(
        include_str!("golden/removal-v1.json"),
        RuntimeRemoval::SCHEMA,
        RuntimeRemoval::validate,
    );
    assert_top_level_fixture::<RuntimeLogQuery>(
        include_str!("golden/log-query-v1.json"),
        RuntimeLogQuery::SCHEMA,
        RuntimeLogQuery::validate,
    );
    assert_top_level_fixture::<RuntimeLogChunk>(
        include_str!("golden/log-chunk-v1.json"),
        RuntimeLogChunk::SCHEMA,
        RuntimeLogChunk::validate,
    );
    assert_top_level_fixture::<RuntimeExecRequest>(
        include_str!("golden/exec-request-v1.json"),
        RuntimeExecRequest::SCHEMA,
        RuntimeExecRequest::validate,
    );
    assert_top_level_fixture::<RuntimeExecResult>(
        include_str!("golden/exec-result-v1.json"),
        RuntimeExecResult::SCHEMA,
        RuntimeExecResult::validate,
    );
    assert_top_level_fixture::<RuntimeUnitRecord>(
        include_str!("golden/unit-record-v2.json"),
        RuntimeUnitRecord::SCHEMA,
        RuntimeUnitRecord::validate,
    );
    assert_top_level_fixture::<RuntimeRequestReceipt>(
        include_str!("golden/request-receipt-v2.json"),
        RuntimeRequestReceipt::SCHEMA,
        RuntimeRequestReceipt::validate,
    );
}

#[test]
fn ct_digest_001_golden_request_and_spec_digests_are_stable() {
    let unit: RuntimeUnitSpec =
        serde_json::from_str(include_str!("golden/unit-spec-v3.json")).unwrap();
    assert_eq!(
        unit.digest().unwrap(),
        "sha256:2d45e69ff2dd43f933cc5b733eda674c30048578c27b5b5bfe1503dbc19d0723"
    );

    let apply: RuntimeApplyRequest =
        serde_json::from_str(include_str!("golden/apply-request-v1.json")).unwrap();
    assert_eq!(
        apply.spec.digest().unwrap(),
        "sha256:77f470a135d8cc7adf4ea1d265f20ffbae030c8b148f0ffa83ef09be35802595"
    );
    assert_eq!(
        apply.digest().unwrap(),
        "sha256:98b0962e04dd1bcc1f8aa4f95ba43919247bf829b9e398a3dc261cfe474f8777"
    );

    let exec: RuntimeExecRequest =
        serde_json::from_str(include_str!("golden/exec-request-v1.json")).unwrap();
    assert_eq!(
        exec.digest().unwrap(),
        "sha256:861f6cad44261c49a89b5cd12920ca3977d833b5c9143d748db945a08939bad0"
    );
}
