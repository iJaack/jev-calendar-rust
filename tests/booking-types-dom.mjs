// Test-only DOM surface for executing the complete shipped frontend without an
// external browser/dependency. It implements native radio/form semantics used by
// these tests; it contains no booking-type rendering or submission logic.
export function testDocument(html) {
 const nodes=[];
 class Element {
  constructor(tag='div') { this.tagName=tag.toUpperCase();this.attrs={};this.children=[];this.listeners={};this.parentElement=null;this._text='';nodes.push(this); }
  setAttribute(k,v){this.attrs[k]=String(v);}
  getAttribute(k){return this.attrs[k]??null;}
  removeAttribute(k){delete this.attrs[k];}
  get id(){return this.attrs.id||'';} set id(v){this.attrs.id=v;}
  get name(){return this.attrs.name||'';} set name(v){this.attrs.name=v;}
  get type(){return this.attrs.type||'';} set type(v){this.attrs.type=v;}
  get value(){return this.attrs.value||'';} set value(v){this.attrs.value=String(v);}
  get className(){return this.attrs.class||'';} set className(v){this.attrs.class=v;}
  get hidden(){return 'hidden' in this.attrs;} set hidden(v){if(v)this.attrs.hidden='';else delete this.attrs.hidden;}
  get required(){return 'required' in this.attrs;} set required(v){if(v)this.attrs.required='';else delete this.attrs.required;}
  get checked(){return 'checked' in this.attrs;} set checked(v){
   if(v){if(this.type==='radio')for(const n of nodes)if(n!==this&&n.type==='radio'&&n.name===this.name)delete n.attrs.checked;this.attrs.checked='';}
   else delete this.attrs.checked;
  }
  get dataset(){return new Proxy({}, {get:(_,k)=>this.attrs['data-'+String(k).replace(/[A-Z]/g,c=>'-'+c.toLowerCase())],set:(_,k,v)=>{this.attrs['data-'+k]=v;return true;}});}
  get classList(){const self=this;const set=()=>new Set(self.className.split(/\s+/).filter(Boolean));return {
   contains:v=>set().has(v),add(...v){self.className=[...new Set([...set(),...v])].join(' ');},remove(...v){self.className=[...set()].filter(x=>!v.includes(x)).join(' ');},
   toggle(v,on){const has=set().has(v),want=on??!has;if(want)this.add(v);else this.remove(v);return want;}
  };}
  append(...children){for(let child of children){if(typeof child==='string'){const text=new Element('#text');text._text=child;child=text;}child.parentElement=this;this.children.push(child);}}
  appendChild(child){this.append(child);return child;}
  replaceChildren(...children){for(const c of this.children)c.parentElement=null;this.children=[];this._text='';this.append(...children);}
  get textContent(){return this._text+this.children.map(c=>c.textContent).join('');} set textContent(v){this.replaceChildren();this._text=String(v);}
  set innerHTML(v){this.replaceChildren();parse(String(v),this);} get innerHTML(){return this.textContent;}
  addEventListener(event,fn){(this.listeners[event]??=[]).push(fn);}
  focus(){} scrollIntoView(){}
  matches(selector){
   selector=selector.trim();
   if(selector.includes(','))return selector.split(',').some(s=>this.matches(s));
   const parts=selector.split(/\s+(?![^\[]*\])/);
   if(parts.length>1){if(!this.matches(parts.pop()))return false;let parent=this.parentElement;while(parent){if(parent.matches(parts.join(' ')))return true;parent=parent.parentElement;}return false;}
   if(selector.includes(':checked')&&!this.checked)return false;
   if(selector.includes(':required')&&!this.required)return false;
   selector=selector.replace(/:(checked|required)/g,'');
   const tag=selector.match(/^[a-z][\w-]*/i)?.[0];if(tag&&this.tagName!==tag.toUpperCase())return false;
   for(const m of selector.matchAll(/#([\w-]+)|\.([\w-]+)|\[([\w-]+)(?:=["']?([^\]"']+)["']?)?\]/g)) {
    if(m[1]&&this.id!==m[1])return false;if(m[2]&&!this.classList.contains(m[2]))return false;
    if(m[3]&&(m[4]===undefined?this.getAttribute(m[3])===null:this.getAttribute(m[3])!==m[4]))return false;
   }
   return true;
  }
  querySelectorAll(selector){return this.children.flatMap(c=>[...(c.matches(selector)?[c]:[]),...c.querySelectorAll(selector)]);}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
  closest(selector){for(let n=this;n;n=n.parentElement)if(n.matches(selector))return n;return null;}
  checkValidity(){return this.querySelectorAll('input').every(n=>!n.required||(n.type==='radio'?this.querySelectorAll('input[type="radio"]').some(r=>r.name===n.name&&r.checked):Boolean(n.value)));}
  reportValidity(){return this.checkValidity();}
  click(){if(this.type==='radio')this.checked=true;for(const fn of this.listeners.click||[])fn({target:this});for(const fn of this.listeners.change||[])fn({target:this});}
 }
 function parse(markup,parent) {
  const stack=[parent],voids=new Set(['INPUT','BR','HR','IMG','META','LINK','AREA','BASE','SOURCE','WBR']);
  for(const token of markup.matchAll(/<!--[\s\S]*?-->|<![^>]*>|<\/[^>]+>|<[^>]+>|[^<]+/g)) {
   const text=token[0];if(text.startsWith('<!'))continue;
   if(text.startsWith('</')){const tag=text.match(/^<\/([\w-]+)/)?.[1].toUpperCase();while(stack.length>1){const last=stack.pop();if(last.tagName===tag)break;}continue;}
   if(text.startsWith('<')){const tag=text.match(/^<([\w-]+)/)?.[1];if(!tag)continue;const el=new Element(tag);
    for(const m of text.slice(tag.length+1,-1).matchAll(/([\w:-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g))el.setAttribute(m[1],m[2]??m[3]??m[4]??'');
    stack.at(-1).append(el);if(!voids.has(el.tagName)&&!text.endsWith('/>'))stack.push(el);
   }else{const el=new Element('#text');el._text=text;stack.at(-1).append(el);}
  }
 }
 const document=new Element('document');parse(html,document);document.visibilityState='hidden';
 document.createElement=tag=>new Element(tag);document.createTextNode=text=>{const n=new Element('#text');n.textContent=text;return n;};
 document.getElementById=id=>document.querySelector('#'+id);
 return document;
}
