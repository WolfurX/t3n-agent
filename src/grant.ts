// Data-owner grant: the customer (USER_KEY) authorizes the agent to run the
// AR contract's functions and lets its reminder emails egress to the gateway
// host. Signed by the USER — not the agent, not the tenant.
//
// Usage: npx tsx src/grant.ts <agentDid>
//        npx tsx src/grant.ts --self        (self-grant: user invokes directly)
import "./env.js";
import { readFile } from "fs/promises";
import { getNodeUrl, getContractVersion } from "@terminal3/t3n-sdk";
import { connect } from "./session.js";
import { requireEnv } from "./env.js";

const arg = process.argv[2];
if (!arg) {
  console.error("Usage: npx tsx src/grant.ts <agentDid>|--self");
  process.exit(1);
}

const state = JSON.parse(
  await readFile(new URL("../setup-state.json", import.meta.url), "utf8"),
);
const gatewayHost = new URL(requireEnv("EMAIL_ENDPOINT")).hostname;

const { client: userClient, did: userDid } = await connect(requireEnv("USER_KEY"));
console.log("Connected as data owner:", userDid);

const agentDid = arg === "--self" ? userDid : arg;
const scriptVersion = await getContractVersion(getNodeUrl(), state.scriptName);
const userContractVersion = await getContractVersion(getNodeUrl(), "tee:user/contracts");

await userClient.execute({
  contract_id: "tee:user/contracts",
  contract_version: userContractVersion,
  function_name: "agent-auth-update",
  input: {
    agents: [
      {
        agentDid,
        scripts: [
          {
            scriptName: state.scriptName,
            versionReq: scriptVersion,
            functions: ["upsert-invoice", "list-invoices", "send-reminder"],
            allowedHosts: [gatewayHost],
          },
        ],
      },
    ],
  },
});
console.log(
  `Granted ${agentDid === userDid ? "self" : agentDid} → ${state.scriptName} v${scriptVersion}, egress: ${gatewayHost}`,
);
process.exit(0);
