import test from 'node:test';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const {Receiver,send,waitForCapacity}=createRequire(import.meta.url)('../peer-transport-v5.js');
class Channel extends EventTarget {readyState='open';bufferedAmount=0;messages=[];send(raw){this.messages.push(JSON.parse(raw));}}
test('wire transfer round-trips unicode and reverse-order duplicate chunks',async()=>{
 const channel=new Channel(),receiver=new Receiver();const payload={text:'á😀漢'.repeat(20000)};
 assert.equal(await send(channel,payload),true);
 receiver.accept('peer',channel.messages.shift());let result;
 for(const message of channel.messages.reverse()){const value=receiver.accept('peer',message);if(value)result=value;receiver.accept('peer',message);}
 assert.deepEqual(result,payload);assert.equal(receiver.reserved,0);receiver.clear();
});
test('receiver bounds aggregate reservations and expires idle streams',()=>{
 let now=0;const receiver=new Receiver({maxChars:100,now:()=>now,ttlMs:10});
 receiver.accept('a',{t:'start',id:'one',total:1,chars:60});receiver.accept('b',{t:'start',id:'two',total:1,chars:60});
 assert.equal(receiver.reserved,60);now=11;receiver.prune();assert.equal(receiver.reserved,0);receiver.clear();
});
test('fractional lengths and overflowing chunks cannot allocate or complete a transfer',()=>{
 const receiver=new Receiver();receiver.accept('p',{t:'start',id:'bad',total:1.5,chars:5});assert.equal(receiver.reserved,0);
 receiver.accept('p',{t:'start',id:'x',total:1,chars:4});assert.equal(receiver.accept('p',{t:'chunk',id:'x',index:0,data:'123456'}),null);assert.equal(receiver.reserved,0);receiver.clear();
});
test('inconsistent duplicates and peer closure discard partial snapshots',()=>{
 const receiver=new Receiver();receiver.accept('p',{t:'start',id:'x',total:2,chars:4});receiver.accept('p',{t:'chunk',id:'x',index:0,data:'ab'});
 receiver.accept('p',{t:'chunk',id:'x',index:0,data:'ac'});assert.equal(receiver.reserved,0);
 receiver.accept('q',{t:'start',id:'y',total:1,chars:4});receiver.dropPeer('q');assert.equal(receiver.reserved,0);receiver.clear();
});
test('backpressure applies before the first frame and closure rejects immediately',async()=>{
 const channel=new Channel();channel.bufferedAmount=100000;
 const pending=send(channel,{text:'x'});await Promise.resolve();assert.equal(channel.messages.length,0);
 const rejected=assert.rejects(pending,/closed/);channel.readyState='closed';channel.dispatchEvent(new Event('close'));await rejected;
});
test('a replaced document cannot send the next frame after draining',async()=>{
 const channel=new Channel();channel.bufferedAmount=100000;let valid=true;
 const pending=send(channel,{text:'x'},{assertCurrent:()=>{if(!valid)throw Error('session expired');}});
 const rejected=assert.rejects(pending,/session expired/);valid=false;channel.bufferedAmount=0;channel.dispatchEvent(new Event('bufferedamountlow'));await rejected;assert.equal(channel.messages.length,0);
});
