---
layout: default
title: gRPC Output
---

# gRPC Output

Handlers can deliver events via gRPC instead of HTTP by setting `type: grpc`.

## Configuration

```yaml
handlers:
  grpc-handler:
    source: app
    events: [order.created]
    url: http://grpc-service:50051
    type: grpc
```

qhook calls the `qhook.v1.EventReceiver/Deliver` unary RPC method.

## Proto Definition

The service definition is at [`proto/qhook.proto`](https://github.com/totte-dev/qhook/blob/main/proto/qhook.proto). Use it to generate server stubs in your language.

```protobuf
service EventReceiver {
  rpc Deliver(DeliverRequest) returns (DeliverResponse);
}

message DeliverRequest {
  string event_id = 1;
  string event_type = 2;
  string handler = 3;
  string payload = 4;
  map<string, string> metadata = 5;
  int32 attempt = 6;
}

message DeliverResponse {
  bool success = 1;
  string message = 2;
}
```

## Request Fields

| Field | Description |
|-------|-------------|
| `event_id` | ULID of the original event |
| `event_type` | Event type (from CloudEvents header, payload, or URL path) |
| `handler` | Handler name from config |
| `payload` | JSON payload (original or transformed) |
| `metadata` | CloudEvents headers (`ce-*`) and `content-type` |
| `attempt` | Current attempt number (1-based) |

## Response Handling

| Response | qhook behavior |
|----------|---------------|
| `success: true` | Job marked completed |
| `success: false` | Job retried with exponential backoff (same as HTTP non-2xx) |
| Connection error | Job retried with exponential backoff |

## Implementing a gRPC Server

### Go

```bash
protoc --go_out=. --go-grpc_out=. proto/qhook.proto
```

```go
type server struct {
    pb.UnimplementedEventReceiverServer
}

func (s *server) Deliver(ctx context.Context, req *pb.DeliverRequest) (*pb.DeliverResponse, error) {
    log.Printf("Received event %s: %s", req.EventType, req.Payload)
    // Process the event...
    return &pb.DeliverResponse{Success: true, Message: "ok"}, nil
}
```

### Python

```bash
pip install grpcio-tools
python -m grpc_tools.protoc -I. --python_out=. --grpc_python_out=. proto/qhook.proto
```

```python
class EventReceiverServicer(qhook_pb2_grpc.EventReceiverServicer):
    def Deliver(self, request, context):
        print(f"Received event {request.event_type}: {request.payload}")
        # Process the event...
        return qhook_pb2.DeliverResponse(success=True, message="ok")
```

## Notes

- gRPC channels are created lazily (on first delivery) and reused across deliveries to the same handler.
- Filter and transform work the same way as HTTP handlers.
- Retry settings (`retry`, `rate_limit`) apply identically.
- CloudEvents metadata is forwarded via the `metadata` map field.
