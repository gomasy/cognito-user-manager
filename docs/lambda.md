# Deploying to Lambda

Function URL behind CloudFront, with the URL locked to `AWS_IAM` so only the
distribution can invoke it.

## One binary, either host

The `lambda` cargo feature is additive: it *adds* the Lambda runtime rather
than replacing the server. Which one serves is decided at startup from
`AWS_LAMBDA_RUNTIME_API`, an environment variable the Lambda runtime sets for
every function it starts and nothing else does.

| `AWS_LAMBDA_RUNTIME_API` | Built with `lambda` | Result |
| --- | --- | --- |
| set | yes | `lambda_http::run` takes the router |
| unset | yes | the listener binds `BIND_ADDR`, unchanged |
| unset | no | the listener binds `BIND_ADDR`, unchanged |
| set | no | exits with a message naming the missing feature, rather than binding a port Lambda will never call |

A binary built with `--features lambda` therefore still runs locally under
`cargo run`, and `--all-features` yields the most capable build rather than a
Lambda-only one. Below `main` nothing knows where it is running: the same tower
service is either handed to the runtime or wrapped in a listener. Logging
follows the same switch, dropping timestamps and ANSI on Lambda because
CloudWatch adds its own.

The cost is that `lambda_http` is linked into that binary even when it serves
locally. Build without the feature for a server-only deployment.

## Why not a REST API

Sign-in sets three cookies at once (`cum_id`, `cum_at`, `cum_rt`). The API
Gateway **REST** payload format cannot carry more than one `Set-Cookie`, so two
of them are dropped and the browser lands in the exact failure the
`SECURE_COOKIES` note in the README describes: sign-in succeeds and the very
next request is unauthenticated. Function URLs and API Gateway **HTTP APIs**
use payload v2, which has a dedicated `cookies` array; `lambda_http` fills it in.

## Build

```bash
cargo install cargo-lambda            # once; brings its own zig-based linker
cargo lambda build --release --arm64 --features lambda
```

`rustls + ring` is already pinned in `Cargo.toml`, so nothing here wants cmake
or a system C toolchain.

## Package

The binary reads `front/dist` and `front/locales` relative to its working
directory, which on Lambda is `/var/task`. Keeping the same relative layout in
the zip means the static file code needs no change at all.

`locales/*.yml` is **not** included: `rust_i18n::i18n!` embeds those catalogs
into the binary at build time. Only `front/locales/*.json`, which the browser
fetches at runtime, has to ship.

```bash
cd front && npm run build && cd ..
cargo lambda build --release --arm64 --features lambda

rm -rf pkg function.zip
mkdir -p pkg/front
cp target/lambda/cognito-user-manager/bootstrap pkg/
cp -r front/dist front/locales pkg/front/
(cd pkg && zip -qr ../function.zip .)
```

In CI, where `.git` may be absent, pass `GIT_HASH` and `BUILD_DATE` as
environment variables; `build.rs` prefers them over shelling out to git. The
result is what `/api/public` reports and the sign-in screen shows.

## IAM role

Save the policy from the README's "Cognito configuration" section as
`iam-policy.json`, with `Resource` scoped to the one user pool ARN.

```bash
ACCOUNT=$(aws sts get-caller-identity --query Account --output text)

aws iam create-role --role-name cognito-user-manager \
  --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}'

aws iam attach-role-policy --role-name cognito-user-manager \
  --policy-arn arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole

aws iam put-role-policy --role-name cognito-user-manager \
  --policy-name cognito-admin --policy-document file://iam-policy.json
```

## Function

```bash
aws lambda create-function \
  --function-name cognito-user-manager \
  --runtime provided.al2023 \
  --architectures arm64 \
  --handler bootstrap \
  --role "arn:aws:iam::${ACCOUNT}:role/cognito-user-manager" \
  --zip-file fileb://function.zip \
  --timeout 30 \
  --memory-size 512 \
  --environment 'Variables={COGNITO_USER_POOL_ID=ap-northeast-1_xxxxxxxxx,COGNITO_CLIENT_ID=xxxxxxxxxxxx,COGNITO_ADMIN_GROUP=admin,SECURE_COOKIES=1}'
```

### Environment variables

The README's table changes on Lambda:

| Variable | On Lambda |
| --- | --- |
| `AWS_REGION` | Set by the runtime. Leave it out; `Config::from_env` is satisfied for free |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | Leave out. The default chain picks up the execution role |
| `BIND_ADDR` | Unused |
| `SECURE_COOKIES` | **Set to `1`.** See below |
| `COGNITO_CLIENT_SECRET` | Only if the app client has one; prefer Secrets Manager |

`SECURE_COOKIES=1` is not optional here. `AllViewerExceptHostHeader` forwards
every viewer header, so a client can send its own `X-Forwarded-Proto: http` on
top of the one the Function URL adds, and `request_is_https` reads the first
entry of that list. Pinning the setting takes the header out of the decision
entirely, which is right anyway behind a distribution that redirects to HTTPS.

## Function URL

`AWS_IAM` means the URL answers nothing without a SigV4 signature, so it cannot
be reached directly once CloudFront is in front of it.

```bash
aws lambda create-function-url-config \
  --function-name cognito-user-manager \
  --auth-type AWS_IAM \
  --query FunctionUrl --output text
```

Note the host (the URL without `https://` and without the trailing `/`) —
`cloudfront.json` needs it below.

## CloudFront

```bash
OAC_ID=$(aws cloudfront create-origin-access-control \
  --origin-access-control-config '{
    "Name": "cognito-user-manager",
    "OriginAccessControlOriginType": "lambda",
    "SigningBehavior": "always",
    "SigningProtocol": "sigv4"
  }' --query 'OriginAccessControl.Id' --output text)
```

Write `cloudfront.json`, substituting the function URL host and `$OAC_ID`:

```json
{
  "CallerReference": "cognito-user-manager-1",
  "Comment": "cognito-user-manager",
  "Enabled": true,
  "HttpVersion": "http2and3",
  "Origins": {
    "Quantity": 1,
    "Items": [{
      "Id": "lambda",
      "DomainName": "REPLACE.lambda-url.ap-northeast-1.on.aws",
      "OriginAccessControlId": "REPLACE_OAC_ID",
      "CustomOriginConfig": {
        "HTTPPort": 80,
        "HTTPSPort": 443,
        "OriginProtocolPolicy": "https-only",
        "OriginSslProtocols": { "Quantity": 1, "Items": ["TLSv1.2"] }
      }
    }]
  },
  "DefaultCacheBehavior": {
    "TargetOriginId": "lambda",
    "ViewerProtocolPolicy": "redirect-to-https",
    "AllowedMethods": {
      "Quantity": 7,
      "Items": ["GET", "HEAD", "OPTIONS", "PUT", "PATCH", "POST", "DELETE"],
      "CachedMethods": { "Quantity": 2, "Items": ["GET", "HEAD"] }
    },
    "CachePolicyId": "4135ea2d-6df8-44a3-9df3-4b5a84be39ad",
    "OriginRequestPolicyId": "b689b0a8-53d0-40ab-baf2-68738e2966ac"
  },
  "CacheBehaviors": {
    "Quantity": 2,
    "Items": [
      {
        "PathPattern": "/front.*",
        "TargetOriginId": "lambda",
        "ViewerProtocolPolicy": "redirect-to-https",
        "AllowedMethods": {
          "Quantity": 2,
          "Items": ["GET", "HEAD"],
          "CachedMethods": { "Quantity": 2, "Items": ["GET", "HEAD"] }
        },
        "Compress": true,
        "CachePolicyId": "658327ea-f89d-4fab-a63d-7e88639e58f6"
      },
      {
        "PathPattern": "/locales/*",
        "TargetOriginId": "lambda",
        "ViewerProtocolPolicy": "redirect-to-https",
        "AllowedMethods": {
          "Quantity": 2,
          "Items": ["GET", "HEAD"],
          "CachedMethods": { "Quantity": 2, "Items": ["GET", "HEAD"] }
        },
        "Compress": true,
        "CachePolicyId": "658327ea-f89d-4fab-a63d-7e88639e58f6"
      }
    ]
  }
}
```

```bash
DIST_ID=$(aws cloudfront create-distribution \
  --distribution-config file://cloudfront.json \
  --query 'Distribution.Id' --output text)
```

The three managed policy IDs are, in order: **CachingDisabled**,
**AllViewerExceptHostHeader**, **CachingOptimized**.

Each is load-bearing:

- **CachingDisabled on the default behavior.** `/api/*` is per-user and
  cookie-bearing; caching any of it would serve one admin's session to another.
- **AllViewerExceptHostHeader.** The app reads `Accept` to decide whether a path
  gets the SPA shell or a 404 (`static_files::wants_html`), and reads `Cookie`
  and `X-App-Lang` on every API call, so those have to reach the origin. `Host`
  must *not* be forwarded: the OAC signature is computed over the Function URL
  host, and overriding it breaks SigV4.
- **CachingOptimized on the static behaviors.** It honours the origin's
  `Cache-Control`, which `static_files.rs` already sets correctly — `immutable`
  for the content-hashed bundles, `must-revalidate` plus an ETag for the
  catalogs. No TTL needs configuring on the CloudFront side.

`/front.*` matches Parcel's `front.<hash>.js` and `front.<hash>.css`. If the
entry point is ever renamed, widen the pattern.

Finally, let the distribution invoke the function:

```bash
aws lambda add-permission \
  --function-name cognito-user-manager \
  --statement-id cloudfront-oac \
  --action lambda:InvokeFunctionUrl \
  --principal cloudfront.amazonaws.com \
  --source-arn "arn:aws:cloudfront::${ACCOUNT}:distribution/${DIST_ID}" \
  --function-url-auth-type AWS_IAM
```

## Updating

```bash
cd front && npm run build && cd ..
cargo lambda build --release --arm64 --features lambda
# repackage as above
aws lambda update-function-code --function-name cognito-user-manager \
  --zip-file fileb://function.zip

# Hashed bundles get new names, so only the stable ones need clearing.
aws cloudfront create-invalidation --distribution-id "$DIST_ID" \
  --paths '/' '/index.html' '/locales/*'
```

## Behaviour to expect

- **Caches are per instance.** `SchemaCache` (5 min) and `Jwks` (1 hr) live in
  the process, so every cold start costs one `DescribeUserPool` and one JWKS
  fetch. `Instant` is monotonic and keeps counting while the sandbox is frozen,
  so the TTLs stay honest.
- **The JWKS fetch is blocking** (`ureq` inside `spawn_blocking`) and adds
  roughly 100–200 ms to a cold start. It runs at most once per hour per
  instance, or once a minute while a `kid` stays unknown.
- **Responses are buffered** by `lambda_http`, against a 6 MB limit. The largest
  thing served is the ~220 kB bundle, and only until CloudFront has it cached.
- `POST /api/auth/logout` answers `204`, which passes through unchanged.
