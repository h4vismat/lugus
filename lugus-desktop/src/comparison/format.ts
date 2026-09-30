export function formatReported(value:string):string{const [whole,fraction]=value.split('.');return whole.replace(/\B(?=(\d{3})+(?!\d))/g,',')+(fraction===undefined?'':'.'+fraction);}
export function formatCalculated(display:string|null):string{return display===null?'—':display+'%';}
