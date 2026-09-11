export const element=<K extends keyof HTMLElementTagNameMap>(tag:K,text?:string,className?:string)=>{const node=document.createElement(tag);if(text!==undefined)node.textContent=text;if(className)node.className=className;return node;};
export const byId=<T extends HTMLElement>(id:string)=>document.getElementById(id) as T;
export function markdown(source:string){
 const root=element('div');let paragraph:string[]=[];let list:HTMLUListElement|null=null;
 const inline=(node:HTMLElement,value:string)=>{for(const part of value.split(/(\*\*[^*]+\*\*|`[^`]+`)/g)){if(part.startsWith('**')&&part.endsWith('**'))node.append(element('strong',part.slice(2,-2)));else if(part.startsWith('`')&&part.endsWith('`'))node.append(element('code',part.slice(1,-1)));else node.append(document.createTextNode(part));}};
 const flush=()=>{if(paragraph.length){const p=element('p');inline(p,paragraph.join('\n'));root.append(p);paragraph=[];}list=null;};
 for(const line of source.split('\n')){if(!line.trim()){flush();continue;}if(/^#{1,4} /.test(line)){flush();const h=element('h3');inline(h,line.replace(/^#+ /,''));root.append(h);}else if(/^[-*] /.test(line)){if(paragraph.length)flush();if(!list){list=element('ul');root.append(list);}const li=element('li');inline(li,line.slice(2));list.append(li);}else{if(list)flush();paragraph.push(line);}}
 flush();return root;
}
