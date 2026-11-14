const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
 const browser=await chromium.launch({headless:true});
 try {
  for (const width of [360,1280]) for (const theme of ['light','dark']) {
   const context=await browser.newContext({viewport:{width:1280,height:900}});
   await context.addInitScript(theme=>{if(window===window.top) localStorage.setItem('bridge.theme',theme)},theme);
   const page=await context.newPage();const errors=[];
   page.on('pageerror',e=>errors.push(e.message));
   await page.goto('http://127.0.0.1:1492');
   await page.locator('button[title^="New chat —"]').first().click();
   const card=page.locator('section[data-outgoing-state="draft"]').first();
   await card.waitFor();
   await page.evaluate(async()=>{
    const m=await import('/src/outgoingMock.ts');
    const item=m.mockListOutgoing('session-1').items[0];
    await m.mockUpdateOutgoing(item.id,item.revision,'Hi',false,[]);
   });
   await card.locator('[data-outgoing-preview]').getByText('Hi',{exact:true}).waitFor();
   await page.setViewportSize({width,height:900});
   await card.getByRole('button',{name:'Send',exact:true}).click();
   const dialog=page.getByRole('dialog',{name:'Confirm connector action'});
   await dialog.waitFor();await dialog.scrollIntoViewIfNeeded();
   const bounds=await dialog.evaluate(node=>{
    const card=node.closest('section');const effect=node.querySelector('p');const heading=node.firstElementChild;
    const rect=n=>{const r=n.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom,right:r.right}};
    return{card:rect(card),dialog:rect(node),effect:rect(effect),heading:rect(heading),effectText:effect.textContent,focused:document.activeElement.textContent};
   });
   assert(bounds.heading.y>=bounds.card.y,'heading clipped above card');
   assert(bounds.dialog.bottom<=bounds.card.bottom,'dialog clipped below card');
   assert(bounds.dialog.x>=bounds.card.x&&bounds.dialog.right<=bounds.card.right,'dialog clipped horizontally');
   assert(bounds.effectText.includes('Slack · #dev'),'destination missing');
   assert.equal(bounds.focused.trim(),'Cancel');
   await page.keyboard.press('Tab');assert.equal(await page.locator(':focus').innerText(),'Send');
   await page.keyboard.press('Tab');assert.equal(await page.locator(':focus').innerText(),'Cancel');
   if(width===360) await card.screenshot({path:`testing/screenshots/outgoing-sends-short-${theme}.png`});
   await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
   await card.getByRole('button',{name:'Send',exact:true}).click();
   await dialog.getByRole('button',{name:'Send',exact:true}).click();
   const sent=page.locator('section[data-outgoing-state="sent"]').first();await sent.waitFor();
   assert.equal(await sent.getByRole('link',{name:'View sent message'}).count(),1);
   assert.equal(await sent.getByRole('button',{name:'Send',exact:true}).count(),0);
   assert.deepEqual(errors,[]);
   console.log(JSON.stringify({width,theme,result:'pass',cardHeight:bounds.card.height,dialogHeight:bounds.dialog.height,checks:['short approval fully inside card','destination visible','keyboard focus loop','Escape cancels','approved send has receipt','no browser errors']}));
   await context.close();
  }
 } finally {await browser.close()}
})().catch(e=>{console.error(e);process.exit(1)});
