# Deploying to Lambda

Function URL behind CloudFront, with the URL locked to `AWS_IAM` so only the
distribution can invoke it. Everything below the build is done from the AWS
Management Console, bar one `aws lambda add-permission` the console has no
equivalent for.

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

So a binary built with `--features lambda` still runs locally under `cargo run`,
and logging follows the same switch, dropping timestamps and ANSI on Lambda
because CloudWatch adds its own. The cost is that `lambda_http` is linked in
even when the binary serves locally; build without the feature for a
server-only deployment.

## Why not a REST API

Sign-in sets three cookies at once (`cum_id`, `cum_at`, `cum_rt`). The API
Gateway **REST** payload format carries only one `Set-Cookie`, so two are
dropped: sign-in succeeds and the next request is unauthenticated. Function URLs
and API Gateway **HTTP APIs** use payload v2, which has a dedicated `cookies`
array that `lambda_http` fills in.

## Build

```bash
cargo install cargo-lambda            # once; brings its own zig-based linker
cargo lambda build --release --x86-64 --features lambda
```

`rustls + ring` is pinned in `Cargo.toml`, so nothing here wants cmake or a
system C toolchain.

Nothing in the code is architecture-specific, so `--arm64` works just as well.
The flag and the function's **Architecture** setting have to agree, or the
runtime fails to start with `Runtime.InvalidEntrypoint`.

## Package

The binary reads `front/dist` and `front/locales` relative to its working
directory, which on Lambda is `/var/task`, so the zip keeps the same layout.

`locales/*.yml` is **not** included: `rust_i18n::i18n!` embeds those catalogs
into the binary at build time. Only `front/locales/*.json`, which the browser
fetches at runtime, has to ship.

```bash
cd front && npm run build && cd ..
cargo lambda build --release --x86-64 --features lambda

rm -rf pkg function.zip
mkdir -p pkg/front
cp target/lambda/cognito-user-manager/bootstrap pkg/
cp -r front/dist front/locales pkg/front/
(cd pkg && zip -qr ../function.zip .)
```

In CI, where `.git` may be absent, pass `GIT_HASH` and `BUILD_DATE` as
environment variables; `build.rs` prefers them over shelling out to git. The
result is what `/api/public` reports and the sign-in screen shows.

The zip stays well under the console's 50 MB direct-upload limit.

Tagging a release does all of the above: `.github/workflows/release.yml` runs
these same steps for both architectures and attaches `function-x86_64.zip` and
`function-arm64.zip` to the GitHub release, so a deployment can start at
**Upload from → .zip file** with a downloaded asset.

## Function

**Lambda → Functions → Create function → Author from scratch**

| Field | Value |
| --- | --- |
| Function name | `cognito-user-manager` |
| Runtime | **Provide your own bootstrap on Amazon Linux 2023** |
| Architecture | **x86_64** (match the build) |
| Execution role | **Create a new role with basic Lambda permissions** |

The handler name is not used by an OS-only runtime; it runs whatever `bootstrap`
is at the root of the zip.

Then, on the function page:

1. **Code → Upload from → .zip file** — pick `function.zip`.
2. **Configuration → General configuration → Edit** — memory **512 MB**,
   timeout **30 seconds**.
3. **Configuration → Environment variables → Edit** — see below.

### IAM role

The role created above only carries `AWSLambdaBasicExecutionRole`, which covers
CloudWatch Logs and nothing else.

**Configuration → Permissions → Execution role** — follow the role name into
IAM, then **Add permissions → Create inline policy → JSON**, and paste the
policy from the README's "Cognito configuration" section with `Resource` scoped
to the one user pool ARN.

### Environment variables

The README's table changes on Lambda:

| Variable | On Lambda |
| --- | --- |
| `AWS_REGION` | Set by the runtime, and reserved — the console rejects it. `Config::from_env` is satisfied for free |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` | Leave out. The default chain picks up the execution role |
| `BIND_ADDR` | Unused |
| `SECURE_COOKIES` | **Set to `1`.** See below |
| `COGNITO_CLIENT_SECRET` | Only if the app client has one; prefer Secrets Manager |

So the list to enter is `COGNITO_USER_POOL_ID`, `COGNITO_CLIENT_ID`,
`COGNITO_ADMIN_GROUP` and `SECURE_COOKIES=1`.

`SECURE_COOKIES=1` is not optional here. `AllViewerExceptHostHeader` forwards
every viewer header, so a client can prepend its own `X-Forwarded-Proto: http`
to the one the Function URL adds. Pinning the setting takes the header out of
the decision entirely.

## Function URL

**Configuration → Function URL → Create function URL**, auth type **AWS_IAM**,
which makes the URL answer nothing without a SigV4 signature. CORS stays off —
the browser only ever talks to the CloudFront domain.

Copy the URL. CloudFront wants the host part of it: no `https://`, no trailing
slash — `xxxxxxxx.lambda-url.ap-northeast-1.on.aws`.

## CloudFront

**CloudFront → Create distribution.**

**Origin**

| Field | Value |
| --- | --- |
| Origin domain | the function URL host (type it in if the picker does not offer it) |
| Origin access | **Origin access control settings**, then **Create new OAC** — origin type *Lambda*, signing behavior *Sign requests* |
| Protocol | **HTTPS only** |

**Default cache behavior**

| Field | Value |
| --- | --- |
| Viewer protocol policy | **Redirect HTTP to HTTPS** |
| Allowed methods | **GET, HEAD, OPTIONS, PUT, POST, PATCH, DELETE** |
| Cache policy | **CachingDisabled** |
| Origin request policy | **AllViewerExceptHostHeader** |

Under **Settings**, leave HTTP/3 enabled if offered.

**Two more behaviors** — after the distribution exists, open its **Behaviors**
tab and **Create behavior** twice, once for path pattern `/front.*` and once for
`/locales/*`. Both take the same origin, **Redirect HTTP to HTTPS**, allowed
methods **GET, HEAD**, **Compress objects automatically** on, cache policy
**CachingOptimized**, and no origin request policy.

The three managed policies are each load-bearing:

- **CachingDisabled on the default behavior.** `/api/*` is per-user and
  cookie-bearing; caching any of it would serve one admin's session to another.
- **AllViewerExceptHostHeader.** The app reads `Accept`, `Cookie` and
  `X-App-Lang`, so those have to reach the origin. `Host` must *not* be
  forwarded: the OAC signature is computed over the Function URL host.
- **CachingOptimized on the static behaviors.** It honours the `Cache-Control`
  the origin already sets, so no TTL needs configuring here.

`/front.*` matches Parcel's `front.<hash>.js` and `front.<hash>.css`. If the
entry point is ever renamed, widen the pattern.

### Letting the distribution in

Creating the OAC grants nothing by itself, and a function URL's resource policy
cannot be edited from the Lambda console. Run this from a terminal or from
CloudShell — the distribution page has a **Copy CLI command** button that fills
in the IDs.

```bash
aws lambda add-permission \
  --function-name cognito-user-manager \
  --statement-id cloudfront-oac \
  --action lambda:InvokeFunctionUrl \
  --principal cloudfront.amazonaws.com \
  --source-arn "arn:aws:cloudfront::<account-id>:distribution/<distribution-id>" \
  --function-url-auth-type AWS_IAM
```

Until this exists every request through CloudFront comes back `403`.

### Signing the request body

OAC signs the origin request, but it will not hash a body it is only relaying,
and Lambda rejects unsigned payloads. The **client** has to send the SHA-256 of
the body in `X-Amz-Content-Sha256`; without it every `POST`, `PUT` and `PATCH`
fails with

```
The request signature we calculated does not match the signature you provided.
```

while `GET`s keep working, which makes it look like a signing misconfiguration
rather than a body problem. Sign-in is the first request to hit it.

`payloadHash` in `front/src/api.ts` covers this for the whole app, and is inert
on every other host. Anything talking to this deployment without going through
`api.ts` has to send the header itself.

## Updating

```bash
cd front && npm run build && cd ..
cargo lambda build --release --x86-64 --features lambda
# repackage as above
```

**Code → Upload from → .zip file** with the new `function.zip`, then
**CloudFront → the distribution → Invalidations → Create invalidation** for
`/`, `/index.html` and `/locales/*`. Hashed bundles get new names, so only the
stable paths need clearing.

## Behaviour to expect

- **Caches are per instance.** `SchemaCache` (5 min) and `Jwks` (1 hr) live in
  the process, so every cold start costs one `DescribeUserPool` and one JWKS
  fetch.
- **The JWKS fetch is blocking** (`ureq` inside `spawn_blocking`) and adds
  roughly 100–200 ms to a cold start. It runs at most once per hour per
  instance, or once a minute while a `kid` stays unknown.
- **Responses are buffered** by `lambda_http`, against a 6 MB limit. The largest
  thing served is the ~220 kB bundle, and only until CloudFront has it cached.
- `POST /api/auth/logout` answers `204`, which passes through unchanged.
- **Passkeys are bound to a domain.** The relying party ID on the user pool has
  to be the host the browser sees — the distribution's own
  `d111111abcdef8.cloudfront.net`, or a custom domain in front of it. Adding
  that custom domain later changes the host, and every passkey already
  registered has to be registered again.
