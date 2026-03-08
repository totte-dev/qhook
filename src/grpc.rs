//! gRPC output support for delivering events to gRPC endpoints.
//!
//! Uses the `qhook.v1.EventReceiver/Deliver` RPC method.
//! See `proto/qhook.proto` for the service definition.

use std::collections::HashMap;

use anyhow::Result;
use tonic::transport::Channel;

/// gRPC request message matching `qhook.v1.DeliverRequest`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DeliverRequest {
    #[prost(string, tag = "1")]
    pub event_id: String,
    #[prost(string, tag = "2")]
    pub event_type: String,
    #[prost(string, tag = "3")]
    pub handler: String,
    #[prost(string, tag = "4")]
    pub payload: String,
    #[prost(map = "string, string", tag = "5")]
    pub metadata: HashMap<String, String>,
    #[prost(int32, tag = "6")]
    pub attempt: i32,
}

/// gRPC response message matching `qhook.v1.DeliverResponse`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DeliverResponse {
    #[prost(bool, tag = "1")]
    pub success: bool,
    #[prost(string, tag = "2")]
    pub message: String,
}

/// Create a lazy gRPC channel (connects on first use).
pub fn create_channel(url: &str) -> Result<Channel> {
    let endpoint = Channel::from_shared(url.to_string())?;
    Ok(endpoint.connect_lazy())
}

/// Deliver an event via gRPC unary call.
pub async fn deliver(channel: &Channel, request: DeliverRequest) -> Result<DeliverResponse> {
    let mut client = tonic::client::Grpc::new(channel.clone());
    client.ready().await?;

    let path: http::uri::PathAndQuery = "/qhook.v1.EventReceiver/Deliver"
        .parse()
        .expect("valid gRPC path");

    let codec: tonic::codec::ProstCodec<DeliverRequest, DeliverResponse> =
        tonic::codec::ProstCodec::default();

    let response = client.unary(tonic::Request::new(request), path, codec).await?;
    Ok(response.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deliver_request_encode_decode() {
        use prost::Message;

        let req = DeliverRequest {
            event_id: "evt_123".into(),
            event_type: "order.created".into(),
            handler: "my-handler".into(),
            payload: r#"{"id": 1}"#.into(),
            metadata: HashMap::from([("ce-type".into(), "order.created".into())]),
            attempt: 1,
        };

        let bytes = req.encode_to_vec();
        let decoded = DeliverRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.event_id, "evt_123");
        assert_eq!(decoded.event_type, "order.created");
        assert_eq!(decoded.metadata.get("ce-type").unwrap(), "order.created");
    }

    #[test]
    fn test_deliver_response_encode_decode() {
        use prost::Message;

        let resp = DeliverResponse {
            success: true,
            message: "ok".into(),
        };

        let bytes = resp.encode_to_vec();
        let decoded = DeliverResponse::decode(bytes.as_slice()).unwrap();
        assert!(decoded.success);
        assert_eq!(decoded.message, "ok");
    }

    #[tokio::test]
    async fn test_create_channel() {
        let channel = create_channel("http://localhost:50051");
        assert!(channel.is_ok());
    }

    #[tokio::test]
    async fn test_create_channel_invalid_url() {
        // Empty string is invalid for channel creation
        let channel = create_channel("");
        assert!(channel.is_err());
    }

    #[tokio::test]
    async fn test_deliver_connection_refused() {
        // Connect to a port that nothing is listening on
        let channel = create_channel("http://127.0.0.1:1").unwrap();
        let request = DeliverRequest {
            event_id: "evt_fail".into(),
            event_type: "test".into(),
            handler: "test-handler".into(),
            payload: "{}".into(),
            metadata: HashMap::new(),
            attempt: 1,
        };
        // Should return an error, not panic
        let result = deliver(&channel, request).await;
        assert!(result.is_err());
    }
}
