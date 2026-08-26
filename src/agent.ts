// The AR agent CLI. Authenticates as the agent (AGENT_KEY) and drives the
// z:<tid>:ar contract. `run` is the cron entry point: remind every overdue
// invoice; the contract's own 20h cooldown makes double-fires harmless.
//
// Usage:
//   npx tsx src/agent.ts add <invoice_id> <customer_ref> <amount> <currency> <due_date>
//   npx tsx src/agent.ts list [all|open|overdue]
//   npx tsx src/agent.ts remind <invoice_id>
//   npx tsx src/agent.ts run
import "./env.js";
import { readFile } from "fs/promises";
import { getNodeUrl, getContractVersion } from "@terminal3/t3n-sdk";
import { connect } from "./session.js";
import { requireEnv } from "./env.js";

const state = JSON.parse(
  await readFile(new URL("../setup-state.json", import.meta.url), "utf8"),
);

const keyVar = process.env.AGENT_KEY ? "AGENT_KEY" : "USER_KEY"; // USER_KEY fallback = self-grant demo
const { client, did } = await connect(requireEnv(keyVar));
console.error(`Connected as ${keyVar === "AGENT_KEY" ? "agent" : "data owner (self)"}: ${did}`);

const contractVersion = await getContractVersion(getNodeUrl(), state.scriptName);

async function call(functionName: string, input: unknown): Promise<any> {
  return client.executeAndDecode({
    contract_id: state.scriptName,
    contract_version: contractVersion,
    function_name: functionName,
    input,
  });
}

const [cmd, ...args] = process.argv.slice(2);
switch (cmd) {
  case "add": {
    const [invoice_id, customer_ref, amount, currency, due_date] = args;
    if (!due_date) {
      console.error("Usage: add <invoice_id> <customer_ref> <amount> <currency> <due_date>");
      process.exit(1);
    }
    const out = await call("upsert-invoice", { invoice_id, customer_ref, amount, currency, due_date });
    console.log(JSON.stringify(out));
    break;
  }
  case "list": {
    const out = await call("list-invoices", { filter: args[0] ?? "all" });
    console.log(JSON.stringify(out, null, 2));
    break;
  }
  case "remind": {
    if (!args[0]) {
      console.error("Usage: remind <invoice_id>");
      process.exit(1);
    }
    const out = await call("send-reminder", { invoice_id: args[0] });
    console.log(JSON.stringify(out));
    break;
  }
  case "run": {
    const { invoices } = await call("list-invoices", { filter: "overdue" });
    let sent = 0;
    let skipped = 0;
    for (const inv of invoices) {
      try {
        const out = await call("send-reminder", { invoice_id: inv.invoice_id });
        console.log(`reminded ${inv.invoice_id} (#${out.reminder_count}, HTTP ${out.email_status})`);
        sent++;
      } catch (e: any) {
        // Cooldown refusals are expected on frequent schedules — not failures.
        console.log(`skipped ${inv.invoice_id}: ${e?.message ?? e}`);
        skipped++;
      }
    }
    console.log(`run complete: ${invoices.length} overdue, ${sent} reminded, ${skipped} skipped`);
    break;
  }
  default:
    console.error("Usage: agent.ts add|list|remind|run  (see file header)");
    process.exit(1);
}
process.exit(0);
