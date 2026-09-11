export type Selection={id:string|null;generation:number};
export function acceptsResult(a:Selection,b:Selection){return a.id===b.id && a.generation===b.generation;}
export type ChartValue={date:string;value:string|null};
export function chartGeometry(values:ChartValue[]) {
  const valid=values.flatMap((p,i)=>p.value!==null && Number.isFinite(Number(p.value))?[{...p,value:p.value,n:Number(p.value),i}]:[]);
  if(!valid.length)return {segments:[],points:[],min:0,max:0};
  const min=Math.min(...valid.map(p=>p.n)),max=Math.max(...valid.map(p=>p.n));
  const scale=Math.max(Math.abs(min),Math.abs(max),1);
  const span=max/scale-min/scale;
  const start=Date.parse(values[0].date),end=Date.parse(values[values.length-1].date);
  const points=valid.map(p=>({date:p.date,value:p.value,x:end>start?((Date.parse(p.date)-start)/(end-start))*340+20:190,y:span===0?95:170-((p.n/scale-min/scale)/span)*150,index:p.i}));
  const segments:string[]=[];let previous=-2;
  for(const p of points){const xy=`${p.x.toFixed(2)},${p.y.toFixed(2)}`;if(p.index!==previous+1)segments.push(`M${xy}`);else segments[segments.length-1]+=` L${xy}`;previous=p.index;}
  return {segments,points,min,max};
}
export function exactNumber(value:string|null){if(value===null)return 'Not reported';const [integer,fraction]=value.split('.');return integer.replace(/\B(?=(\d{3})+(?!\d))/g,',')+(fraction===undefined?'':`.${fraction}`);}
export type Identifier={namespace:string;value:string};
export function sameCompany(a?:Identifier,b?:Identifier){return !!a && !!b && a.namespace===b.namespace && a.value===b.value;}
