# Mini API Gateway

基于 Rust + Tower 的高性能 API Gateway，支持日志、限流、CORS、TLS、Prometheus 监控、响应缓存、JWT/OAuth2 认证、Consul/Etcd 服务发现等功能。

## 功能特性

- **HTTP/HTTPS 代理**：基于 Hyper 和 Tower 的高性能反向代理
- **TLS 支持**：使用 rustls 实现 HTTPS，支持客户端证书认证
- **路由**：支持路径匹配、方法过滤、前缀剥离
- **负载均衡**：支持 Round Robin、Random、IP Hash、Least Connections
- **限流**：基于 Governor 的令牌桶限流，支持 IP/Header/全局维度
- **CORS**：完整的跨域资源共享策略支持
- **认证**：JWT 签名验证、OAuth2 集成
- **缓存**：多级缓存（内存 Moka / Redis），支持 TTL 和自定义缓存键策略
- **服务发现**：Consul 健康检查、Etcd v3 键值存储
- **监控**：Prometheus 指标导出（请求数、延迟、错误率、缓存命中率等）
- **连接池**：基于 reqwest 的 HTTP/2 连接池
- **零拷贝**：优化的 body 传输减少内存拷贝
- **日志**：基于 tracing 的结构化日志（JSON/Pretty/Compact）

## 快速开始

### 编译

```bash
cargo build --release
```

### 运行

使用默认配置（带演示路由）：

```bash
cargo run
```

使用自定义配置文件：

```bash
GATEWAY_CONFIG=config.yaml cargo run
```

### 配置文件示例

```yaml
server:
  listen: "0.0.0.0:8080"
  workers: 4
  request_timeout_secs: 30
  keepalive_secs: 75
  max_body_size: 10485760

tls:
  enabled: true
  cert_path: "./certs/cert.pem"
  key_path: "./certs/key.pem"
  client_auth:
    enabled: false
    ca_path: "./certs/ca.pem"

routes:
  - id: api
    path: "/api/**"
    upstream:
      type: static
      url: "http://localhost:3000"
    strip_prefix: "/api"
    retry:
      max_attempts: 3
      backoff_ms: 100
    timeout_ms: 30000
    cache_enabled: true
    auth_required: false

  - id: user_service
    path: "/users/**"
    upstream:
      type: service
      name: "user-service"
      discovery: "consul"
    auth_required: true

  - id: lb_example
    path: "/lb/**"
    upstream:
      type: load_balance
      endpoints:
        - "http://backend1:8080"
        - "http://backend2:8080"
      strategy: round_robin

rate_limit:
  enabled: true
  requests_per_second: 100
  burst_size: 200
  key_strategy: ip

cors:
  enabled: true
  allow_origins:
    - "*"
  allow_methods:
    - "GET"
    - "POST"
    - "PUT"
    - "DELETE"
    - "OPTIONS"
  allow_headers:
    - "*"
  allow_credentials: false
  max_age: 3600

auth:
  enabled: true
  type: jwt
  secret: "your-secret-key"
  issuer: "mini-gateway"
  audience: "api"
  excluded_paths:
    - "/health"
    - "/api/auth/**"

cache:
  enabled: true
  type: memory
  default_ttl_secs: 300
  max_capacity: 10000
  cacheable_methods:
    - "GET"
  cacheable_statuses:
    - 200
    - 301
    - 404
  key_strategy: uri_with_method
  excluded_paths:
    - "/api/auth/**"

metrics:
  enabled: true
  listen: "0.0.0.0:9090"
  endpoint: "/metrics"

discovery:
  enabled: true
  type: consul
  address: "http://localhost:8500"
  datacenter: "dc1"
  refresh_interval_secs: 10

logging:
  level: "info"
  format: json
  output: stdout
  request_id_header: "x-request-id"

pool:
  enabled: true
  max_connections: 100
  idle_timeout_secs: 60
  connection_timeout_ms: 5000
```

### 环境变量

配置也可以通过环境变量覆盖，格式为 `GATEWAY__<SECTION>__<KEY>`：

```bash
GATEWAY__SERVER__LISTEN=0.0.0.0:8080
GATEWAY__RATE_LIMIT__REQUESTS_PER_SECOND=1000
```

### Prometheus 指标

启动后访问 `http://localhost:9090/metrics` 查看：

- `gateway_requests_total` — 总请求数
- `gateway_request_duration_seconds` — 请求延迟
- `gateway_requests_error_total` — 错误响应数
- `gateway_cache_hits_total` — 缓存命中数
- `gateway_cache_misses_total` — 缓存未命中数
- `gateway_rate_limited_total` — 限流触发数
- `gateway_active_connections` — 活跃连接数

## 生成 TLS 证书（开发测试）

```bash
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes
```

## 项目结构

```
src/
├── main.rs              # 入口
├── lib.rs               # 库模块声明
├── config.rs            # 配置管理
├── error.rs             # 错误类型
├── gateway.rs           # 核心 Gateway 服务
├── router.rs            # 路由匹配
├── proxy.rs             # HTTP 代理客户端
├── tls.rs               # TLS/HTTPS 支持
├── pool.rs              # 连接池与零拷贝工具
├── metrics.rs           # Prometheus 指标导出
├── middleware/          # Tower 中间件
│   ├── auth.rs          # JWT/OAuth2 认证
│   ├── cache.rs         # 响应缓存
│   ├── cors.rs          # CORS 策略
│   ├── logging.rs       # 请求日志
│   ├── metrics.rs       # 指标收集
│   └── rate_limit.rs    # 限流
├── cache/               # 缓存后端
│   ├── memory.rs        # 内存缓存 (Moka)
│   └── redis.rs         # Redis 缓存
└── discovery/           # 服务发现
    ├── consul.rs        # Consul 集成
    └── etcd.rs          # Etcd 集成
```

## License

MIT
