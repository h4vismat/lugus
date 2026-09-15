const {chromium}=require(process.env.LUGUS_PLAYWRIGHT_MODULE||'playwright');
const fs=require('node:fs'),http=require('node:http'),cp=require('node:child_process'),readline=require('node:readline'),path=require('node:path'),assert=require('node:assert/strict');
const dir=fs.mkdtempSync(path.join(require('node:os').tmpdir(),'lugus-live-dashboard-'));
const pluginRoot=path.resolve(__dirname,'../../lugus-financial/plugins/yfinance');
fs.writeFileSync(path.join(dir,'plugin.json'),JSON.stringify({id:'yfinance',version:'0.3.0',protocol_version:1,command:path.join(pluginRoot,'.venv/bin/python'),args:[path.join(pluginRoot,'main.py')]}));
fs.writeFileSync(path.join(dir,'app.json'),JSON.stringify({financial_path:'financial.sqlite',application_path:'app.sqlite',providers:[{instance_id:'qa',manifest:'plugin.json',active:true,config:{}}]}));
fs.writeFileSync(path.join(dir,'desktop.json'),JSON.stringify({application_config:'app.json'}));
let child,pending=[];function start(offline=false){child=cp.spawn(path.resolve(__dirname,'../src-tauri/target/debug/examples/portfolio_qa'),[path.join(dir,'desktop.json'),...(offline?[]:['--online'])],{stdio:['pipe','pipe','inherit']});readline.createInterface({input:child.stdout}).on('line',line=>pending.shift()?.(JSON.parse(line)));}
function rpc(payload){return new Promise(resolve=>{pending.push(resolve);child.stdin.write(JSON.stringify(payload)+'\n');});}
async function command(command){const r=await rpc({op:'portfolio',command});if(r.error)throw r.error;return r.value;}
const root=path.resolve(__dirname,'../dist');const server=http.createServer(async(req,res)=>{if(req.url==='/rpc'){let body='';for await(const chunk of req)body+=chunk;res.setHeader('content-type','application/json');res.end(JSON.stringify(await rpc(JSON.parse(body))));return;}const file=path.resolve(root,'.'+(req.url==='/'?'/index.html':req.url.split('?')[0]));if(!file.startsWith(root+path.sep)){res.statusCode=403;res.end();return;}try{res.setHeader('content-type',file.endsWith('.js')?'text/javascript':file.endsWith('.css')?'text/css':'text/html');res.end(fs.readFileSync(file));}catch{res.statusCode=404;res.end();}});
// Opt-in live check: creates only disposable, explicitly labelled QA data.
(async()=>{let browser;const errors=[];try{
 start();let revision='0',portfolio=null;
 const mutate=async mutation=>{const r=await command({kind:'execute',request:{request_id:crypto.randomUUID(),portfolio_id:portfolio,expected_revision:revision,mutation}});portfolio=r.portfolio_id;revision=r.revision;return r;};
 await mutate({kind:'create_portfolio',name:'QA — live source validation'});
 const instrument=(await mutate({kind:'create_instrument',name:'Apple QA holding',symbol:'AAPL',asset_kind:'stock'})).instrument_ids[0];
 await mutate({kind:'bind_instrument',instrument_id:instrument,instance_id:'qa',native_id:{namespace:'yahoo:symbol',value:'AAPL'}});
 await mutate({kind:'create_account',name:'QA synthetic opening',start:'2026-08-03',opening:{kind:'existing',cash:'1000',lots:[{id:'qa-lot',instrument_id:instrument,acquired:'2026-08-03',tie_order:0,quantity:'10',basis:'2000',simplified:true,date_assumed:false}]},events:[]});
 const end=new Intl.DateTimeFormat('en-CA',{timeZone:'America/New_York',year:'numeric',month:'2-digit',day:'2-digit'}).format(new Date());
 let result=await command({kind:'history_start',request:{request_id:'live-check',portfolio_id:portfolio,account_id:null,expected_revision:revision,range:{start:end.slice(0,4)+'-01-01',end},refresh:'force'}});
 const deadline=Date.now()+300000;
 while(result.status==='running'&&Date.now()<deadline){await new Promise(r=>setTimeout(r,250));result=await command({kind:'history_status',portfolio_id:portfolio,id:result.id});}
 assert.ok(['complete','partial'].includes(result.status),JSON.stringify(result));
 assert.notEqual(result.summary.portfolio_return_percent,null);assert.notEqual(result.summary.benchmark_return_percent,null);
 const evidence=await command({kind:'history_evidence',portfolio_id:portfolio,id:result.id,offset:0});assert.equal(evidence.items.length,2);
 let points=[],offset=0;do{const page=await command({kind:'history_read',portfolio_id:portfolio,id:result.id,offset});points.push(...page.items);offset=page.next_offset;}while(offset!==null);
 assert.equal(points.length,result.row_count);
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));browser=await chromium.launch({headless:true,executablePath:process.env.LUGUS_CHROMIUM,args:['--no-sandbox']});const page=await browser.newPage({viewport:{width:1440,height:1100}});page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{window.__TAURI_INTERNALS__={invoke:async(cmd,args)=>{const r=await fetch('/rpc',{method:'POST',body:args.payload}).then(r=>r.json());if(r.error)throw r.error;return r.value;}};});
 await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.getByRole('button',{name:'Portfolio',exact:true}).click();await page.locator('.portfolio-series-line').first().waitFor({timeout:15000});
 await page.getByText('Methodology & sources',{exact:true}).click();await page.getByRole('link',{name:'Source price history'}).first().waitFor();await page.screenshot({path:path.join(dir,'live-dashboard.png'),fullPage:true});
 assert.deepEqual(errors,[]);const report={passed:true,artifact_dir:dir,result,evidence,first:points[0],last:points.at(-1),page_errors:errors};fs.writeFileSync(path.join(dir,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({passed:true,artifact_dir:dir,result_id:result.id,rows:points.length,summary:result.summary}));
 }catch(e){console.error(e);console.error(dir);process.exitCode=1;}finally{if(browser)await browser.close();server.close();child?.stdin.end();}})();
