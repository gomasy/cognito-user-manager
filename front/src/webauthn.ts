/**
 * The browser half of passkeys.
 *
 * Cognito speaks the WebAuthn structures as JSON with every binary member
 * base64url-encoded, which is what `parseCreationOptionsFromJSON` and
 * `PublicKeyCredential.toJSON` produce. Both are recent, so the same
 * conversion is written out by hand for a browser that has neither.
 */
import { t } from "./i18n";

type Json = Record<string, any>;

/**
 * Whether this browser can hold a passkey at all. `PublicKeyCredential` is
 * undefined outside a secure context, so plain HTTP answers false here.
 */
export function isSupported(): boolean {
  return (
    typeof window.PublicKeyCredential !== "undefined" &&
    typeof navigator.credentials?.create === "function" &&
    typeof navigator.credentials?.get === "function"
  );
}

function decode(value: string): Uint8Array {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded.padEnd(Math.ceil(padded.length / 4) * 4, "="));
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function encode(value: ArrayBuffer): string {
  const binary = String.fromCharCode(...new Uint8Array(value));
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** The binary members of a credential descriptor, in either list. */
function descriptors(list: unknown): Json[] | undefined {
  if (!Array.isArray(list)) return undefined;
  return list.map((entry: Json) => ({ ...entry, id: decode(entry.id) }));
}

function creationOptions(options: Json): PublicKeyCredentialCreationOptions {
  return {
    ...options,
    challenge: decode(options.challenge),
    user: { ...options.user, id: decode(options.user.id) },
    excludeCredentials: descriptors(options.excludeCredentials),
  } as unknown as PublicKeyCredentialCreationOptions;
}

function requestOptions(options: Json): PublicKeyCredentialRequestOptions {
  return {
    ...options,
    challenge: decode(options.challenge),
    allowCredentials: descriptors(options.allowCredentials),
  } as unknown as PublicKeyCredentialRequestOptions;
}

/** What the authenticator answered, in the JSON Cognito verifies. */
function serialize(credential: PublicKeyCredential): Json {
  const toJson = (credential as unknown as { toJSON?: () => Json }).toJSON;
  if (typeof toJson === "function") return toJson.call(credential);

  const response = credential.response;
  const body: Json = { clientDataJSON: encode(response.clientDataJSON) };

  if (response instanceof AuthenticatorAttestationResponse) {
    body.attestationObject = encode(response.attestationObject);
    body.transports = response.getTransports?.() ?? [];
  } else if (response instanceof AuthenticatorAssertionResponse) {
    body.authenticatorData = encode(response.authenticatorData);
    body.signature = encode(response.signature);
    body.userHandle = response.userHandle ? encode(response.userHandle) : null;
  }

  return {
    id: credential.id,
    rawId: encode(credential.rawId),
    type: credential.type,
    authenticatorAttachment: credential.authenticatorAttachment,
    clientExtensionResults: credential.getClientExtensionResults(),
    response: body,
  };
}

/** The static parsers, when this browser has them. */
function parser(name: "parseCreationOptionsFromJSON" | "parseRequestOptionsFromJSON") {
  const statics = window.PublicKeyCredential as unknown as Record<
    string,
    ((options: Json) => PublicKeyCredentialCreationOptions & PublicKeyCredentialRequestOptions) | undefined
  >;
  return typeof statics?.[name] === "function" ? statics[name] : undefined;
}

/**
 * Cancelling the prompt is an ordinary thing to do, and arrives as a
 * DOMException whose wording is neither localized nor meant to be read.
 */
async function prompt(request: () => Promise<Credential | null>): Promise<Json> {
  if (!isSupported()) throw new Error(t("passkey.unsupported"));

  let credential: Credential | null;
  try {
    credential = await request();
  } catch (error) {
    if (error instanceof DOMException) {
      if (error.name === "NotAllowedError" || error.name === "AbortError") {
        throw new Error(t("passkey.cancelled"));
      }
      throw new Error(`${t("passkey.failed")} (${error.name})`);
    }
    throw error;
  }

  if (!credential) throw new Error(t("passkey.cancelled"));
  return serialize(credential as PublicKeyCredential);
}

/** Makes a passkey from the options Cognito issued. */
export function create(options: Json): Promise<Json> {
  const parse = parser("parseCreationOptionsFromJSON");
  // Cognito sends the options bare; a wrapped one is accepted all the same.
  const publicKey = (options.publicKey ?? options) as Json;
  return prompt(() =>
    navigator.credentials.create({
      publicKey: parse ? parse(publicKey) : creationOptions(publicKey),
    }),
  );
}

/** Signs the challenge Cognito issued with a passkey the user already has. */
export function authenticate(options: Json): Promise<Json> {
  const parse = parser("parseRequestOptionsFromJSON");
  const publicKey = (options.publicKey ?? options) as Json;
  return prompt(() =>
    navigator.credentials.get({
      publicKey: parse ? parse(publicKey) : requestOptions(publicKey),
    }),
  );
}
