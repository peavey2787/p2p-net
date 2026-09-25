import init, { WasmNode } from "./pkg/p2p_net.js";

await init();
let node = null;
const $ = (id) => document.getElementById(id);
const log = (value) => { $("log").textContent += `${typeof value === "string" ? value : JSON.stringify(value)}\n`; };
const active = (yes) => {
  $("start").disabled = yes; $("stop").disabled = !yes;
  $("connect").disabled = !yes; $("broadcast").disabled = !yes;
};

$("start").onclick = async () => {
  try {
    node = await WasmNode.start({ network_id: Number($("network").value), profile: "auto" }, $("profile").value);
    active(true);
    log(`peer=${node.peerId()}`);
    log(await node.localBinding());
    const events = node.subscribeEvents();
    (async () => { while (node) log(await events.recv()); })().catch((e) => log(e));
  } catch (e) { log(e); }
};
$("connect").onclick = async () => { try { await node.connectPeer($("addr").value); log("connected/dial accepted"); } catch (e) { log(e); } };
$("broadcast").onclick = async () => { try { await node.broadcast($("topic").value, new TextEncoder().encode($("payload").value)); log("broadcast accepted"); } catch (e) { log(e); } };
$("stop").onclick = async () => { try { await node.shutdown(); node = null; active(false); log("shutdown"); } catch (e) { log(e); } };
