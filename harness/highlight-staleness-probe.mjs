// Regression probe for the code-fence highlight lifecycle.
//
// The bug class: CodeBlock keeps the previous highlight while a streaming
// fence grows (deliberate — clearing it strobes), so a React instance reused
// for DIFFERENT content must actively clear, or the old fence's markup renders
// over the new fence's text. That failure is invisible to the unit suite
// (vitest runs with no DOM and cannot observe effects), so it is pinned here.
//
//   node harness/highlight-staleness-probe.mjs
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium, webkit } from "playwright";
const root = new URL("../", import.meta.url).pathname;
const port = await new Promise((r,j)=>{const s=createServer();s.once("error",j);s.listen(0,"127.0.0.1",()=>{const{port}=s.address();s.close(()=>r(port));});});
const url=`http://127.0.0.1:${port}/`;
const vite=spawn("node",["node_modules/vite/bin/vite.js","--host","127.0.0.1","--port",String(port),"--strictPort"],{cwd:`${root}app`,env:{...process.env,VITE_PERF_HOOKS:"1",VITE_PRODUCT_DEV_AUTH:"1"},stdio:["ignore","pipe","pipe"]});
for(let i=0;i<200;i++){try{if((await fetch(url)).ok)break;}catch{}await sleep(150);}
const seed=`const scope=encodeURIComponent("id:perf-qa");
localStorage.setItem("agent-desktop.dev-account",JSON.stringify({user:{id:"perf-qa",name:"Perf QA",method:"local"}}));
localStorage.setItem('agent-desktop:local-agent:'+scope,JSON.stringify({cwd:"/tmp",model:"local-model",reasoningEffort:"high"}));
localStorage.setItem('agent-desktop:project-context:'+scope,JSON.stringify({cwd:"/tmp"}));`;
const b=await (process.env.ENGINE === "webkit" ? webkit : chromium).launch({headless:true});
const failures=[];
try{
  const ctx=await b.newContext({viewport:{width:1360,height:880}});
  await ctx.addInitScript(seed);
  const p=await ctx.newPage();
  const errs=[];p.on("pageerror",e=>errs.push(String(e.message)));
  await p.goto(url,{waitUntil:"domcontentloaded"});
  await p.getByLabel("Message Clark Code").waitFor({state:"visible",timeout:60000});
  await p.waitForFunction(()=>"__clarkPerf" in window,null,{timeout:30000});

  const setFence = (text) => p.evaluate((body)=>{
    const st=window.__agentDesktopStore;
    const snap=st.getState().snapshot;
    st.setState({
      session:{id:"stale-qa",provider:"local",collaboration_mode:"default",
        capabilities:{streaming:true,permissions:true,fs:true,terminal:true,load_session:true,modes:["default"],collaboration_modes:["default"]}},
      connecting:false,opening:null,
      snapshot:{...snap,session:"stale-qa",
        timeline:[
          {item:"message",run:"r1",role:"user",blocks:[{type:"text",text:"show me"}]},
          {item:"message",run:"r1",role:"agent",phase:"final_answer",blocks:[{type:"text",text:body}]},
        ],
        runs:{r1:{id:"r1",status:"done",outcome:{status:"done"},checkpoint:"qa"}}},
    });
  }, text);
  const read = () => p.evaluate(()=>({
    shiki: document.querySelectorAll(".shiki-host").length,
    shikiText: document.querySelector(".shiki-host")?.textContent?.slice(0,80) ?? null,
    preText: [...document.querySelectorAll("pre")].map(x=>x.textContent?.slice(0,80)),
  }));

  // 1. TS fence highlights.
  await setFence("```ts\nconst alpha = 1;\nconst beta = 2;\n```");
  // A cold module worker must fetch its engine, themes, and first grammar.
  // Wait for the actual output; a fixed sleep conflates readiness with a
  // machine-dependent startup deadline, especially in the WebKit proxy.
  await p.waitForFunction(
    () => document.querySelector(".shiki-host")?.textContent?.includes("alpha"),
    null,
    { timeout: 15_000 },
  );
  let s1 = await read();
  if (s1.shiki < 1 || !s1.shikiText?.includes("alpha")) failures.push(`step1: expected highlighted alpha fence, got ${JSON.stringify(s1)}`);

  // 2. GROWTH: same fence + suffix -> old highlight may remain visible
  //    (anti-strobe) but must settle to include the suffix.
  await setFence("```ts\nconst alpha = 1;\nconst beta = 2;\nconst gamma = 3;\n```");
  await sleep(900);
  const s2 = await read();
  if (s2.shiki < 1 || !s2.shikiText?.includes("gamma")) failures.push(`step2 growth: expected settled highlight incl gamma, got ${JSON.stringify(s2)}`);

  // 3. REPLACEMENT with a plain (no-language) fence: the old TS highlight must
  //    disappear immediately — this was the stale-HTML bug.
  await setFence("```\nplain replacement content here\n```");
  await sleep(120); // well under the highlight quiet period
  const s3 = await read();
  if (s3.shiki !== 0) failures.push(`step3 plain replacement: stale shiki-host still present: ${JSON.stringify(s3)}`);
  if (!s3.preText.some(t=>t?.includes("plain replacement"))) failures.push(`step3: new plain content not rendered: ${JSON.stringify(s3)}`);

  // 4. REPLACEMENT with a different TS fence: old markup must not linger with
  //    the old content; final state highlights the new content.
  await setFence("```ts\nconst delta = 4;\n```");
  await sleep(60);
  const s4early = await read();
  if (s4early.shikiText?.includes("alpha")) failures.push(`step4 early: old fence's markup lingered: ${JSON.stringify(s4early)}`);
  await sleep(900);
  const s4 = await read();
  if (s4.shiki < 1 || !s4.shikiText?.includes("delta")) failures.push(`step4 settled: expected delta highlighted, got ${JSON.stringify(s4)}`);

  if (errs.length) failures.push(`page errors: ${errs.slice(0,3).join(" | ")}`);
  for (const f of failures) console.log("FAIL:", f);
  console.log(failures.length===0 ? "stale-highlight checks: ALL PASS" : `${failures.length} failure(s)`);
  process.exitCode = failures.length===0 ? 0 : 1;
}finally{await b.close();vite.kill("SIGTERM");}
