// One-time tenant setup (idempotent — safe to re-run when re-deploying):
//   1. authenticate as the tenant (T3N_API_KEY from the claim page)
//   2. register the compiled contract WASM under the tail "ar"
//   3. create the `invoices` + `secrets` KV maps, ACL'd to the new contract id
//   4. seed the email gateway endpoint/key into `secrets`
//
// Usage: npx tsx src/setup.ts <contract-version>   e.g. 0.1.0
// Re-registering the same tail requires a strictly higher version.
import "./env.js";
import { readFile, writeFile } from "fs/promises";
import { TenantClient, getNodeUrl } from "@terminal3/t3n-sdk";
import { connect } from "./session.js";
import { requireEnv } from "./env.js";

const WASM_PATH = new URL(
  "../contract/target/wasm32-wasip2/release/z_tenant_ar.wasm",
  import.meta.url,
);
const CONTRACT_TAIL = "ar";

const version = process.argv[2];
if (!version) {
  console.error("Usage: npx tsx src/setup.ts <contract-version>   e.g. 0.1.0");
  process.exit(1);
}

const { client: t3n, did: tenantDid } = await connect(requireEnv("T3N_API_KEY"));
console.log("Connected as tenant:", tenantDid);

const tenant = new TenantClient({ t3n, baseUrl: getNodeUrl(), tenantDid });
await tenant.tenant.me();
console.log("TenantClient ready.");

const wasmBytes = await readFile(WASM_PATH);
const result = await tenant.contracts.register({
  tail: CONTRACT_TAIL,
  version,
  wasm: wasmBytes,
});
const contractId = result.contract_id;
const scriptName = `z:${tenantDid.slice("did:t3n:".length)}:${CONTRACT_TAIL}`;
console.log(`Registered ${scriptName} v${version} as contract id ${contractId}`);

// Maps: create is not idempotent across re-registrations (a new contract id
// needs the ACL re-pointed), so fall back to update when the map exists.
for (const tail of ["invoices", "secrets"]) {
  const acl = {
    tail,
    visibility: "private" as const,
    readers: { only: [contractId] }, // REQUIRED — the kv-governor denies reads when omitted
    writers: { only: [contractId] },
  };
  try {
    await tenant.maps.create(acl);
    console.log(`Created map ${tail} (reader/writer: contract ${contractId})`);
  } catch (e: any) {
    if (String(e?.message ?? e).includes("already exists")) {
      await tenant.maps.update(acl);
      console.log(`Map ${tail} exists — ACL re-pointed at contract ${contractId}`);
    } else {
      throw e;
    }
  }
}

// Seed the email gateway config. Control-plane writes bypass the writers ACL.
const secrets: Array<[string, string | undefined]> = [
  ["email_endpoint", requireEnv("EMAIL_ENDPOINT")],
  ["email_api_key", process.env.EMAIL_API_KEY],
  ["email_from", process.env.EMAIL_FROM],
];
for (const [key, value] of secrets) {
  if (!value) continue;
  await tenant.executeControl("map-entry-set", {
    map_name: tenant.canonicalName("secrets"),
    key,
    value,
  });
  console.log(`Seeded secrets/${key}`);
}

// Later steps (grant, agent) need these; keep a record because there is no
// API to fetch a tail's current contract_id after re-registering.
const state = { tenantDid, scriptName, contractId, version };
await writeFile(new URL("../setup-state.json", import.meta.url), JSON.stringify(state, null, 2));
console.log("Wrote setup-state.json:", JSON.stringify(state));
process.exit(0);
