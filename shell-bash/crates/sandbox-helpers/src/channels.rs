//! Drain and fill `StreamChannelRef`s using the iii SDK's
//! `ChannelReader` and `ChannelWriter`.

use iii_sdk::{ChannelReader, ChannelWriter, IIIError, StreamChannelRef, III};

pub async fn drain_ref(iii: &III, channel_ref: &StreamChannelRef) -> Result<Vec<u8>, IIIError> {
    let reader = ChannelReader::new(iii.address(), channel_ref);
    let bytes = reader.read_all().await?;
    Ok(bytes)
}

pub async fn fill_ref(
    iii: &III,
    channel_ref: &StreamChannelRef,
    bytes: &[u8],
) -> Result<(), IIIError> {
    let writer = ChannelWriter::new(iii.address(), channel_ref);
    writer.write(bytes).await?;
    writer.close().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_channel_ref_round_trips_through_serde() {
        // The iii engine returns channel refs from `sandbox::fs::read` as JSON
        // with the shape `{ channel_id, access_key, direction }`, and accepts
        // the same shape on `sandbox::fs::write`. Round-tripping through
        // serde here pins that contract: if the SDK ever changes field names
        // or casing, this test fails before the worker hits the engine.
        let r: StreamChannelRef = serde_json::from_value(json!({
            "channel_id": "cid",
            "access_key": "key",
            "direction": "read",
        }))
        .expect("StreamChannelRef deserializes from sandbox::fs::read response shape");
        assert_eq!(r.channel_id, "cid");
        assert_eq!(r.access_key, "key");

        let back = serde_json::to_value(&r)
            .expect("StreamChannelRef serializes for sandbox::fs::write payload");
        assert_eq!(back["channel_id"], "cid");
        assert_eq!(back["access_key"], "key");
        assert_eq!(back["direction"], "read");
    }
}
