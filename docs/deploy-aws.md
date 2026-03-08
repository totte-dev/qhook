# Deploy qhook to AWS

## Prerequisites

- Docker image: `qhook:latest`
- Server port: `8888`
- Config file: `/data/qhook.yaml`
- DB: SQLite (standalone) or PostgreSQL (recommended for production)

---

## 1. ECS Fargate (Recommended)

### 1.1 Push Image to ECR

```bash
aws ecr create-repository --repository-name qhook

AWS_ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)
AWS_REGION=ap-northeast-1

aws ecr get-login-password --region $AWS_REGION | \
  docker login --username AWS --password-stdin $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com

docker tag qhook:latest $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com/qhook:latest
docker push $AWS_ACCOUNT_ID.dkr.ecr.$AWS_REGION.amazonaws.com/qhook:latest
```

### 1.2 Task Definition

```json
{
  "family": "qhook",
  "networkMode": "awsvpc",
  "requiresCompatibilities": ["FARGATE"],
  "cpu": "256",
  "memory": "512",
  "executionRoleArn": "arn:aws:iam::ACCOUNT_ID:role/ecsTaskExecutionRole",
  "containerDefinitions": [
    {
      "name": "qhook",
      "image": "ACCOUNT_ID.dkr.ecr.ap-northeast-1.amazonaws.com/qhook:latest",
      "portMappings": [
        {
          "containerPort": 8888,
          "protocol": "tcp"
        }
      ],
      "environment": [
        { "name": "RUST_LOG", "value": "qhook=info" },
        { "name": "DATABASE_URL", "value": "postgres://qhook:password@rds-host:5432/qhook" }
      ],
      "mountPoints": [
        {
          "sourceVolume": "qhook-config",
          "containerPath": "/data",
          "readOnly": true
        }
      ],
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/ecs/qhook",
          "awslogs-region": "ap-northeast-1",
          "awslogs-stream-prefix": "qhook"
        }
      },
      "essential": true
    }
  ],
  "volumes": [
    {
      "name": "qhook-config",
      "efsVolumeConfiguration": {
        "fileSystemId": "fs-xxxxxxxxx",
        "rootDirectory": "/qhook"
      }
    }
  ]
}
```

> **Config file placement:** Put `qhook.yaml` on EFS, or remove the `mountPoints` / `volumes` sections if you pass everything via environment variables.

### 1.3 Create Service (with ALB)

```bash
# Create target group (port 8888, health check /health)
aws elbv2 create-target-group \
  --name qhook-tg \
  --protocol HTTP \
  --port 8888 \
  --vpc-id vpc-xxxxxxxx \
  --target-type ip \
  --health-check-path /health

# Create ECS service
aws ecs create-service \
  --cluster default \
  --service-name qhook \
  --task-definition qhook \
  --desired-count 2 \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[subnet-aaa,subnet-bbb],securityGroups=[sg-xxx],assignPublicIp=DISABLED}" \
  --load-balancers "targetGroupArn=arn:aws:elasticloadbalancing:...:targetgroup/qhook-tg/xxx,containerName=qhook,containerPort=8888"
```

### 1.4 RDS PostgreSQL

- Allow port `5432` from the ECS task security group in the RDS security group
- Pass `DATABASE_URL` via task definition environment variables or Secrets Manager

```json
{
  "name": "DATABASE_URL",
  "valueFrom": "arn:aws:secretsmanager:ap-northeast-1:ACCOUNT_ID:secret:qhook/database-url"
}
```

When using Secrets Manager, add it to the `secrets` field and grant `secretsmanager:GetSecretValue` permission to `executionRoleArn`.

### 1.5 Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `DATABASE_URL` | DB connection string | `postgres://user:pass@host:5432/qhook` |
| `RUST_LOG` | Log level | `qhook=info` |
| `QHOOK_CONFIG` | Config file path (default: `/data/qhook.yaml`) | `/data/qhook.yaml` |

---

## 2. EC2 (Single Instance)

For small-scale or test environments.

### 2.1 Docker + docker-compose

`docker-compose.prod.yaml`:

```yaml
services:
  qhook:
    image: qhook:latest
    restart: always
    ports:
      - "127.0.0.1:8888:8888"
    volumes:
      - ./qhook.yaml:/data/qhook.yaml:ro
      - qhook-data:/data/db
    environment:
      - RUST_LOG=qhook=info
      - DATABASE_URL=postgres://qhook:password@db:5432/qhook

  db:
    image: postgres:16-alpine
    restart: always
    volumes:
      - pgdata:/var/lib/postgresql/data
    environment:
      - POSTGRES_USER=qhook
      - POSTGRES_PASSWORD=password
      - POSTGRES_DB=qhook

volumes:
  qhook-data:
  pgdata:
```

```bash
docker compose -f docker-compose.prod.yaml up -d
```

### 2.2 systemd Service

For running the binary directly without Docker Compose.

`/etc/systemd/system/qhook.service`:

```ini
[Unit]
Description=qhook event gateway
After=network.target postgresql.service

[Service]
Type=simple
User=qhook
Group=qhook
ExecStart=/usr/local/bin/qhook start --config /data/qhook.yaml
Restart=always
RestartSec=5
Environment=RUST_LOG=qhook=info
Environment=DATABASE_URL=postgres://qhook:password@localhost:5432/qhook
WorkingDirectory=/data
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now qhook
```

### 2.3 Nginx Reverse Proxy

`/etc/nginx/sites-available/qhook`:

```nginx
server {
    listen 80;
    server_name webhook.example.com;

    # Redirect to HTTPS (after obtaining cert via Let's Encrypt)
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl http2;
    server_name webhook.example.com;

    ssl_certificate     /etc/letsencrypt/live/webhook.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/webhook.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8888;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Webhook payloads can be large
        client_max_body_size 10m;
    }
}
```

```bash
sudo ln -s /etc/nginx/sites-available/qhook /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

---

## 3. Lambda + API Gateway (Not Recommended)

qhook is not suitable for serverless environments:

- **Long-running queue worker**: qhook continuously polls for queued jobs and performs retry deliveries. Lambda's execution time limit (max 15 min) and cold starts are fundamentally incompatible.
- **Persistent DB connections**: qhook maintains a connection pool, which is inefficient with Lambda's lifecycle.
- **Cost**: Continuous polling on Lambda means instances running 24/7, making it more expensive than Fargate or EC2.

A split architecture (Lambda for receiving, separate service for queue processing) is possible but goes against qhook's design philosophy of a single self-contained binary.

**Conclusion:** Run qhook as a long-running process on ECS Fargate or EC2.
