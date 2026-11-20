# OIDC login (Dex / Keycloak)

ORBIT uses standard OIDC discovery, Authorization Code + PKCE and JWKS validation. No
provider-specific code is required beyond the issuer URL and client registration.

## Required configuration

| Setting | Env | Example |
|---|---|---|
| Auth mode | `ORBIT_AUTH_MODE` | `oidc` |
| Issuer | `ORBIT_OIDC_ISSUER` | `https://dex.example.com` |
| Audience | `ORBIT_OIDC_AUDIENCE` | `orbit` |
| Client ID | `ORBIT_OIDC_CLIENT_ID` | `orbit` |
| Client secret | `ORBIT_OIDC_CLIENT_SECRET` | optional for public PKCE clients |
| Redirect URI | `ORBIT_OIDC_REDIRECT_URI` | `http://orbit.example.com/auth/callback` |
| Scopes | `ORBIT_OIDC_SCOPES` | `openid profile email groups` |
| JWKS | `ORBIT_OIDC_JWKS_URL` | issuer keys URL |
| Role claim | `ORBIT_OIDC_ROLE_CLAIM` | `groups` |
| Identity/email claim | `ORBIT_OIDC_EMAIL_CLAIM` | `email` (or `name`) |
| Group mappings | `ORBIT_OIDC_GROUP_ROLES` | `orbit-admins=admin,orbit-operators=operator` (or JSON) |
| Sync roles | `ORBIT_OIDC_SYNC_ROLES` | `false` |

`ORBIT_OIDC_CLIENT_ID` defaults to the audience. The redirect URI must be registered
exactly at the provider.

The API fetches:

```text
<issuer>/.well-known/openid-configuration
```

for `authorization_endpoint` and `token_endpoint`. The browser never receives the client
secret.

## Dex example

Dex static client:

```yaml
staticClients:
  - id: orbit
    name: ORBIT
    secret: optional-for-confidential-client
    redirectURIs:
      - http://orbit.example.com/auth/callback
```

Set:

```text
ORBIT_OIDC_ISSUER=https://dex.example.com
ORBIT_OIDC_AUDIENCE=orbit
ORBIT_OIDC_CLIENT_ID=orbit
ORBIT_OIDC_REDIRECT_URI=http://orbit.example.com/auth/callback
ORBIT_OIDC_JWKS_URL=https://dex.example.com/keys
```

Dex access tokens may be opaque. ORBIT uses the `id_token` from the token response as the
browser bearer token and validates it against JWKS.

## Keycloak example

Create an OpenID Connect client:

- Client ID: `orbit`
- Client authentication: off for public PKCE client, or on with a client secret
- Standard flow: enabled
- Valid redirect URI: `http://orbit.example.com/auth/callback`
- Web origins: `http://orbit.example.com`

Issuer:

```text
https://keycloak.example.com/realms/<realm>
```

Set the same five variables plus `ORBIT_OIDC_JWKS_URL` if your deployment does not expose
discovery at the issuer. ORBIT’s API validates the JWT audience against
`ORBIT_OIDC_AUDIENCE` and auto-provisions users when enabled.

## Group-based roles

ORBIT can map IdP groups to built-in roles. Add the claim and mappings:

```yaml
auth:
  oidc:
    scopes: "openid profile email groups"
    defaultRole: viewer
    roleClaim: groups
    groupRoles:
      orbit-admins: admin
      orbit-operators: operator
    syncRoles: true
```

- The IdP must include the claim in the ID token (Dex connector group support or a
  Keycloak group-membership mapper).
- The most privileged matching role wins (`owner > admin > operator > viewer`).
- `syncRoles=false` applies the mapped role only when a user is first auto-provisioned.
- `syncRoles=true` updates an existing user's stored role on login only when the token
  has groups **and** at least one matches `groupRoles`; the role then becomes the highest
  mapped one. A token without the claim, or whose groups match no mapping, leaves the
  stored role (including manually assigned and custom roles) unchanged. A matching mapping
  does override manual role changes in ORBIT.
- Only groups listed in `groupRoles` grant anything. `defaultRole` is used only when a new
  user is auto-provisioned without a matching group; it is never applied by sync.

Equivalent environment variables: `ORBIT_OIDC_ROLE_CLAIM`,
`ORBIT_OIDC_GROUP_ROLES` (`group=role,group=role` or JSON), `ORBIT_OIDC_SYNC_ROLES`.

Group mappings target built-in roles. Custom roles (`docs/development/rbac.md`) are
assigned manually for now; group-to-custom-role mapping is a follow-up.

OIDC groups can also add users to access teams; see
[oidc-team-sync.md](oidc-team-sync.md).

## Configure from the UI

Once the platform is reachable, OIDC can also be configured under **Settings → OIDC / SSO**
(`GET`/`PUT /api/v1/settings/oidc`). Values are stored in `platform_settings`, merged over
the environment configuration in OIDC mode, and applied when the API starts — restart the
API after saving. `ORBIT_OIDC_CLIENT_SECRET` is never stored by the UI; keep
confidential-client secrets in the deployment Secret.

## Web flow

```text
/login
  → GET /api/v1/auth/oidc/config
  → redirect to authorization_endpoint with PKCE
  → /auth/callback
  → POST /api/v1/auth/oidc/token { code, codeVerifier }
  → API exchanges code at token_endpoint
  → browser stores id_token as orbit.access_token
  → API requests use Authorization: Bearer <id_token>
```

Logout currently clears the local token. Provider end-session logout and refresh-token
rotation are not implemented yet.

## Helm

```yaml
auth:
  mode: oidc
  oidc:
    issuer: https://dex.example.com
    audience: orbit
    clientId: orbit
    redirectUri: http://orbit.example.com/auth/callback
    jwksUrl: https://dex.example.com/keys
    scopes: "openid profile email"
    autoProvision: true
    defaultRole: viewer
secrets:
  oidcClientSecret: ""
```

For a public PKCE client leave `oidcClientSecret` empty. For a confidential client set
it; the API adds it to the server-side token exchange.
