# Aster Team contracts

`contracts/` contains the repository-wide sources of truth that are consumed by
more than one process, language, or generated client.

- `openapi/` describes HTTP interfaces.
- `schemas/` describes signed and cross-process JSON documents.
- `catalogs/` contains registries from which language-specific constants are generated.
- `test-vectors/` proves that independent implementations serialize and verify the same bytes.

Generated Rust, Go, and TypeScript files stay beside their consumers and must not
be edited manually. CI regenerates them and rejects a dirty diff. Database rows and
module-internal domain models are not public contracts and remain under `customer/`
or `operations/`.

## Product capabilities and entitlements

`catalogs/product-capabilities.yaml` is the single registry for capability IDs,
quota IDs, dependency metadata and labels. `npm run generate:contracts` generates
Rust types in license-core, the Operations Go catalog, TypeScript types for the
Customer SDK, Operations console and website, and the entitlement JSON schema.
`npm run verify:contracts` rejects stale outputs. Do not edit generated files.

The catalog describes available software capabilities, not commercial plans.
Prices, edition names, selected features and quota values belong to Operations
plan versions. New plan creation validates the selection and fixes its dependency
closure; the form reads the generated catalog instead of maintaining another list.
The initial three capability IDs preserve the existing gateway/member/runner
identifiers. Adding a paid capability must not silently include it in old grants.

The same catalog's `operations` list describes composite business requirements.
It generates `BusinessOperationId::required_capabilities()` in Rust and matching
Go/TypeScript definitions. OAuth enrollment, manual credential refresh and model
synchronization require gateway plus runner; stored-account/model management
retains gateway alone. Operations forms show missing requirements without adding
features to a plan. Customer buttons project these requirements, while the server
checks all requirements against one verified License snapshot before preparation
and again at task dispatch. Automatic refresh during model execution retains its
existing runner requirement and does not acquire the administrator's gateway rule.
An operation is not a new signed entitlement, price or database grant. Adding an
operation alone does not change the entitlement schema or existing feature grants.
New operations must declare known, nonempty requirements and add their real business
success and denial coverage; this initial list does not claim all routes are mapped.

`schemas/entitlements.v1.schema.yaml` and `test-vectors/entitlements.v1.json`
describe an unsigned entitlement payload, not a replacement for License v1.
Every quota is explicit: `limited` with value 0 forbids use, a positive value is
a finite ceiling, and `unlimited` carries no value. The public Go parser rejects
missing, null, duplicate, unknown and case-variant object fields. JSON Schema
checks object shape and duplicate capability/quota entries; it cannot detect
object keys already collapsed by a generic JSON parser. Rust wire types reject
malformed structure, and callers must additionally invoke
`Entitlements::validate` for complete quotas, dependency closure and uniqueness.
Both backends use the shared vector to check serialization and enforce issuer
ceilings. A validated payload is still not a verified or active license.

The vector's 3/1/1/2 numbers are test data matching the discussed proposal. They
are not production defaults, an approved free plan or a signed free certificate.
Counting semantics, binding and duration must be frozen in the subsequent signed
protocol and plan contract before distribution. Catalog changes must preserve
existing IDs and semantics; revisions require explicit compatibility for historical
plan snapshots and licenses. Never recalculate old entitlements from today's catalog.

The existing signed License v1 wire format is unchanged in this stage. Do not apply the new
registry to historical v1 files without the agreed compatibility mapping. Future
signing and runtime integration must validate payloads, enforce issuer policy,
verify signatures and then evaluate binding, time, roles and resource limits.

## Unified signed License v2

`schemas/license.v2.schema.yaml` describes `{ claims, signature }`. Ed25519 signs
canonical JSON of the entire `claims` object, including its schema, source,
plan ID/version, explicit binding, validity, quota-policy version and complete
entitlements. V2 is a new envelope; it is never obtained by dropping unknown
fields from v1 or treating missing binding fields as unrestricted.

Rust `license-core::v2` and Go `licenseprotocol` validate structure and semantics,
verify the signature and enforce the trusted issuer's allowed sources, binding
modes, expiry modes, features and quota ceilings. A valid signature does not
by itself approve a commercial source, a currently active installation or a
business operation. Runtime binding, time, installation history, role and quota
checks must use the returned authenticated claims in the subsequent integration.

The Rust key collection separates legacy v1 entries from explicitly scoped v2
entries. It rejects a scoped public key reused under any other ID or protocol,
and failed duplicate registration does not replace the prior entry. Go's v2 key
collection has no unscoped/legacy export and similarly refuses public-key aliases.
Do not put the free issuer's public key in the legacy production key configuration.
Issuer metadata must come from the protected trust configuration, never the
customer's license, a business table or the website export. Existing v1-only
aliases are retained for compatibility; they do not admit scoped v2 keys.

The shared v2 vector has four test-only documents: free with a fixed expiry,
free with no expiry, a bound commercial order and a bound approved trial. Tests
compare canonical bytes and signatures in Rust and Go using the explicitly
non-production seed `[42; 32]`. These alternatives do not approve a free expiry
policy or a paid quota plan. `quota_policy_version = 1` reserves the first
counting contract; its business semantics still require D03 confirmation before
runtime integration or issuance. No production v2 certificate is distributed yet.

`schemas/license-trust.v1.schema.yaml` is the shared public issuer-profile contract.
The Operations profile response references this contract; Customer build tooling,
local release and setup use one parser and retain the complete v2 `policy` in
compiled trust. Rust `TrustedLicenseKeys::from_json` requires v2 profiles with
strict field, policy, key and duplicate checks. Missing policy, explicit null and
malformed policies are rejected. Free distribution issuers cannot also authorize
commercial or approved trial sources. Public profiles never contain a private key.
Formal Customer builds prohibit runtime trust overrides.

`test-vectors/license-trust.v1.json` links Operations public-profile serialization,
build normalization and Rust signature/scope verification to the existing license
vectors. The separate `legacy_entry` is a rejection fixture, not accepted trust.
All fixture key material is test-only. Customer product entrypoints, installation
history and Operations fulfillment now use v2; older protocol primitives remain
only for bounded negative and cryptographic tests.

Both parsers reject duplicate/case-variant/missing/null fields, trailing input,
unknown tags and noncanonical Base64. Schema checks wire structure; actual dates,
chronological ordering, integer SemVer bounds, trust and signatures require the
backend validators. The Operations v2 signing adapter freezes its trusted scope at construction and
checks it before signing; application-level source approval remains required.
Runtime use still requires signature, source scope, binding, time, history and
business permission checks; schema validation alone never grants authorization.

## Immutable commercial snapshots

`schemas/commercial.v1.schema.yaml` defines strict plan definitions, complete plan
versions and order snapshots. Operations OpenAPI references these definitions and
the generated entitlement schema; the console receives the same capability and
quota IDs. The schema validates wire structure; Go additionally checks valid
calendar dates, unique selected years, supported SemVer, canonical hashes and
calculated amounts. A content hash identifies content, not source approval.

Annual terms use integer minor units and basis-point factors. Round the final
annual amount × years × factor / 10000 once, half up. Calendar years preserve the
agreed local time and clamp leap-day anniversaries; supported zones are embedded
in the Operations Go binary, so a minimal installation needs no external zoneinfo.
Test fixtures in `test-vectors/plan-definition.v1.json` and `pricing.v1.json` are
examples only: they do not approve production prices, free expiry, tax, support
or quota-counting decisions.

A plan revision appends an immutable version; orders contain the entire selected
version, amount, factor, support reference and start/end dates. Creating an order
does not mark it paid or approve signing. Free and contact offers cannot enter the
fixed-price order path. Idempotent retries retain operation identity and explicit
contract dates. V2 signing may occur after the purchased start, while still before
the fixed expiry; late issuance never extends the purchased end or changes start.

The new `/commercial/*` Operations routes are authenticated internal management
APIs, not website pricing feeds. Read/write permissions are separate for plans
and orders and enforced at the service boundary. Commercial fulfillment, v1
migration and Customer execution remain integration work within the same goal.

`/commercial/quotation-sources/{reference}` resolves an exact catalog revision or
publication ID using order-write permission and the server-configured sales
channel. Its projection contains only the fixed annual plans and the selected
publication identity/deadline; it does not grant publication-management access.
`/commercial/quotation-orders` derives an order from that accepted publication
inside a deadline-bound transaction. Caller-supplied amounts, rights, environment
and approval evidence are rejected. Historical accepted sources remain eligible
only through their own explicit deadline, independent of the current head.

Order schema v2 requires a compact publication receipt inside the canonical
order digest, alongside the full selected plan. V1 manual orders retain their
original encoding and cannot carry a source field. Original-operation recovery
precedes current channel, deadline and customer eligibility checks; the receipt
must match the original actor and every request field, never a rebuilt snapshot
with a new timestamp. Later paid fulfillment must resolve and compare the trusted
original publication as well as the order; a well-formed compact receipt or its
self-hash alone does not prove approval. Manual v1 orders do not imply a verified
website quotation.

## Approved free distributions

`schemas/free-distribution.v1.schema.yaml` defines internal approval requests,
persisted distribution states and public issuer profiles. An approval fixes a
complete free plan version, its hash, start time, actor and reason. Saving a plan
does not approve it. The `prepared` transition freezes the exact claims and key;
`issued` stores the matching signed document and hash. Retrying an interrupted
issuance reuses those bytes and dates, including when the frozen expiry has since
passed; this recovers the original document without granting a new validity term.

Approval and signing require separate permissions and current-password
reauthentication. Authentication fields exist only in the HTTP request, not the
commercial input, snapshot, audit or browser retry journal. Every returned issued
record is checked against its source and trusted signature. Download returns the
exact stored bytes, and the console checks the SHA-256 against both the record
and response header. Public profiles contain SPKI and scoped policy, never keys
capable of signing. Runtime installation, expiry and quota enforcement are still
required before an operational free package is complete.

Contract verification registers relative YAML aliases and compiles every root
schema in strict mode, including contracts without sample vectors. Date-time
formats use `ajv-formats`; vector and behavior tests additionally validate actual
approval and issuance responses. A schema-valid document is not proof of source
approval, signature validity or an active customer installation.


## Explicit installation request v2

`schemas/license-request.v2.schema.yaml` and the shared test vector declare the
actual installation's license schema, capability catalog and quota policy versions
alongside the complete machine request. Go and Rust reject missing, duplicate,
case-variant, unknown, null and unsupported fields. Product versions use SemVer
precedence without build metadata, consistently with Customer policy currentness.
These are compatibility declarations, not authenticated customer identity or rights.
They do not prove that a release has passed installation/runtime acceptance.

The current Customer generator remains v1 until its v2 import, policy and recovery
path is implemented and verified. Legacy requests are never promoted by default
values or a guessed product release number. The old compact QR encoder strictly
requires a v1 request, preventing silent removal of v2 compatibility fields; a new
QR wire contract and Operations importer must be completed before v2 QR delivery.

`schemas/paid-fulfillment.v1.schema.yaml` fixes the complete payment/order source,
trusted approval environment/customer reference, original request JSON, parsed
request/digest and approval identity. Request JSON reaches the strict server parser
as text; do not pre-parse it in the browser and submit a reserialized object.
The approved/prepared/issued states fix claims and original dates. Their domain
validation does not implement permission checks, transactional uniqueness, source
authenticity or signature verification. Paid persistence/API/UI integration must
perform those checks before this record can be delivered. Supplemental issuance,
transfer, renewal and trial conversion require separate approved transitions.

## 功能集合与免费默认值

`catalogs/product-capabilities.yaml` 是功能 ID、功能集合归属、免费默认开放及依赖的唯一来源。`feature_set: standard` 表示标准功能，其他集合是明确注册的可选扩展模块。`free_default` 仅用于运营端准备新套餐的具体功能列表，绝不参与 Customer 动态扩权。

权益的 `features` 始终是具体功能；可选 `feature_sets` 是签名授予的符号集合，不是模式匹配。缺失表示没有集合权益，`null`、未知名称、重复集合均拒绝。空集合在签名规范化中省略，Go/Rust 保持相同规则，原有 v2 向量签名字节不变。只有安装绑定且固定期限的商业来源可以授予集合，免费和试用签发者不得包含集合 ceiling。集合授权必须处于签发者明确授予的同名集合范围，当前全功能列表不能授权未来集合；明确功能可以落在 ceiling 的已授集合内。四类额度仍必须逐项明确声明和比较。

Customer 的签名原文及运营端的套餐/订单快照保留符号集合，运行和展示时才根据编译内置目录展开，不能将展开结果回写为签名输入。增加标准能力是同一目录合同版本内的兼容扩展，不提高 `catalog_version`，不修改已有 ID 和归属；标准集合不能依赖扩展模块，免费默认能力的依赖也必须免费默认开放。新增额度维度或改变现有授权语义属于合同变更，必须设计显式迁移，不能默认无限。

当前只注册标准集合，全部现有能力归属标准集合。新增扩展模块需要先注册稳定集合 ID，运营页面和类型生成后自动列出可选集合；并为实际执行边界声明能力检查。没有全局 `*`、菜单接口自动放行或按数据库标记授予功能。
