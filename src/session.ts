// Shared T3N session construction. Every identity (tenant, agent, data owner)
// authenticates the same way: handshake, authenticate, read the DID back from
// the session — never construct or derive a DID yourself.
import {
  T3nClient,
  setEnvironment,
  loadWasmComponent,
  eth_get_address,
  metamask_sign,
  createEthAuthInput,
  fetchTrustedManifest,
} from "@terminal3/t3n-sdk";

setEnvironment("testnet");

// One crypto component shared by every client in this process.
const wasmComponentPromise = loadWasmComponent();

export async function connect(key: string): Promise<{ client: T3nClient; did: string }> {
  const address = eth_get_address(key);
  const client = new T3nClient({
    trustAnchor: await fetchTrustedManifest("testnet"),
    wasmComponent: await wasmComponentPromise,
    handlers: {
      EthSign: metamask_sign(address, undefined, key),
    },
  });
  await client.handshake();
  const did = await client.authenticate(createEthAuthInput(address));
  return { client, did: did.value };
}
