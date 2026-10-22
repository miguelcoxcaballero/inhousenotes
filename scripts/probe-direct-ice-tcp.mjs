// Diagnostic only: compare direct data channels with UDP allowed vs filtered.
// No STUN/TURN or relay. A local pass is not evidence of NAT traversal.
import { chromium } from '@playwright/test';
const browser = await chromium.launch({ headless: true });
try {
 const page = await browser.newPage();
 const results = await page.evaluate(async () => {
  const results = [];
  for (const tcpOnly of [false, true]) {
   const a = new RTCPeerConnection({ iceServers: [] });
   const b = new RTCPeerConnection({ iceServers: [] });
   try {
    const candidates = [], pending = [];
    for (const [from, to] of [[a,b],[b,a]]) from.onicecandidate = event => {
     const c = event.candidate;
     if (!c) return;
     candidates.push({ protocol: c.protocol, type: c.type, tcpType: c.tcpType });
     if (!tcpOnly || c.protocol === 'tcp') pending.push([to,c.toJSON()]);
    };
    let received = false;
    const channel = a.createDataChannel('ink');
    b.ondatachannel = event => { event.channel.onmessage = () => { received = true; }; };
    const description = pc => ({ type: pc.localDescription.type,
     sdp: pc.localDescription.sdp.split('\r\n').filter(line => !line.startsWith('a=candidate:')).join('\r\n') });
    await a.setLocalDescription(await a.createOffer());
    await b.setRemoteDescription(description(a));
    await b.setLocalDescription(await b.createAnswer());
    await a.setRemoteDescription(description(b));
    const started = performance.now();
    while (performance.now() - started < 6000) {
     while (pending.length) { const [peer,candidate] = pending.shift(); await peer.addIceCandidate(candidate); }
     if (channel.readyState === 'open') channel.send('ink');
     await new Promise(resolve => setTimeout(resolve,50));
     if (received) break;
    }
    results.push({ tcpOnly, received, state: a.connectionState, candidates });
   } finally { a.close(); b.close(); }
  }
  return results;
 });
 console.log(JSON.stringify({ browser: browser.version(), results },null,2));
 if (!results[0].received) process.exitCode = 1;
} finally { await browser.close(); }
