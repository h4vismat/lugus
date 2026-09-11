import type {Dataset,ResearchView} from './types';

// Dataset headers contain JSON values; compare metadata independently of key order.
function sameMetadata(left:unknown,right:unknown):boolean {
 if(left===right)return true;
 if(left===null||right===null||typeof left!=='object'||typeof right!=='object')return false;
 if(Array.isArray(left)!==Array.isArray(right))return false;
 const a=left as Record<string,unknown>,b=right as Record<string,unknown>;
 const keys=Object.keys(a);
 return keys.length===Object.keys(b).length&&keys.every(key=>Object.hasOwn(b,key)&&sameMetadata(a[key],b[key]));
}

/** Read a bounded suffix of an immutable saved dataset, retaining its original header. */
export async function readSavedWindow(
 first:Dataset,
 read:(offset:number)=>Promise<Dataset>,
 maxRows=1000,
 allowSourceError=false,
):Promise<Dataset> {
 if(!Number.isSafeInteger(maxRows)||maxRows<1)throw new Error('Saved data window must have a positive whole-number limit.');
 const total=first.header.row_count;
 if(!Number.isSafeInteger(total)||total<0)throw new Error('Saved dataset has an invalid row count.');
 function validate(page:Dataset,offset:number):number {
  if(page.header.error&&!allowSourceError)throw new Error(page.header.error.message);
  if(!sameMetadata(first.header,page.header))throw new Error('Saved dataset metadata changed while reading its pages.');
  const end=offset+page.rows.length;
  if(end>total||!Number.isSafeInteger(end))throw new Error('Saved dataset page exceeds its row count.');
  if(end<total){
   if(page.rows.length===0||page.next_offset!==end)throw new Error('Saved dataset pagination did not advance consistently.');
  }else if(page.next_offset!==null)throw new Error('Saved dataset pagination continues beyond its row count.');
  return end;
 }
 const firstEnd=validate(first,0);
 const start=Math.max(0,total-maxRows);
 const rows:Dataset['rows']=start<firstEnd?first.rows.slice(start):[];
 let offset=Math.max(start,firstEnd);
 while(offset<total){
  const page=await read(offset);
  const end=validate(page,offset);
  rows.push(...page.rows);
  offset=end;
 }
 return {header:first.header,rows,next_offset:null};
}

/** Facts must be complete; unlike a price chart, dropping old rows drops metrics. */
export async function readSavedFacts(first:Dataset,read:(offset:number)=>Promise<Dataset>):Promise<Dataset> {
 if(first.header.row_count>100_000)throw new Error('The financial listing exceeds the desktop limit of 100,000 observations.');
 return readSavedWindow(first,read,Math.max(1,first.header.row_count),true);
}

/** An explicit selection must resolve exactly; only an absent selection may use a default. */
export function selectedResearch(items:ResearchView[],id:string|null):ResearchView|undefined {
 return id===null?items[0]:items.find(item=>item.view.id===id);
}
