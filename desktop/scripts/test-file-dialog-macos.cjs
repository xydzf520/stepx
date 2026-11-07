#!/usr/bin/env node
// Tests the real JXA entry point with Standard Additions mocked. No native GUI.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const script = fs.readFileSync(path.join(__dirname, '../src/file_dialog_macos.js'), 'utf8');
function invoke(request, selection, failure) {
  const calls = [];
  const app = {};
  for (const method of ['chooseFile', 'chooseFolder', 'chooseFileName']) {
    app[method] = options => {
      calls.push({method, options: JSON.parse(JSON.stringify(options))});
      if (failure) throw failure;
      return selection;
    };
  }
  const context = vm.createContext({Application: {currentApplication: () => app}, Path: p => p});
  vm.runInContext(script, context);
  const result = JSON.parse(context.run([JSON.stringify(request)]));
  return {result, calls, app};
}
const hostile = '中文 " ; do shell script "touch /tmp/never"\n$(never)`never`\\';
let test = invoke({kind:'open_files',title:hostile,location:'/tmp/中文 目录',extensions:['png','pdf']}, ['/tmp/空 格\n换行.pdf', '/tmp/quote".png']);
assert.deepEqual(test.result, {status:'selected',paths:['/tmp/空 格\n换行.pdf', '/tmp/quote".png']});
assert.equal(test.calls[0].method, 'chooseFile');
assert.equal(test.calls[0].options.withPrompt, hostile);
assert.equal(test.calls[0].options.multipleSelectionsAllowed, true);
assert.deepEqual(test.calls[0].options.ofType, ['png','pdf']);
assert.equal(test.calls[0].options.defaultLocation, '/tmp/中文 目录');
assert.equal(test.app.includeStandardAdditions, true);
for (const [kind, method] of [['open_file','chooseFile'],['open_folder','chooseFolder'],['save_file','chooseFileName']]) {
  test = invoke({kind, title:'选择', filename:hostile, extensions:[]}, {toString: () => '/tmp/单项'});
  assert.deepEqual(test.result, {status:'selected',paths:['/tmp/单项']});
  assert.equal(test.calls[0].method, method);
  if (kind === 'save_file') assert.equal(test.calls[0].options.defaultName, hostile);
  if (kind === 'open_file') assert.equal(test.calls[0].options.multipleSelectionsAllowed, false);
}
for (const failure of [{errorNumber:-128},{number:-128}]) {
  assert.deepEqual(invoke({kind:'open_file',extensions:[]}, null, failure).result, {status:'cancelled'});
}
assert.equal(invoke({kind:'open_file',extensions:[]}, null, {message:'permission denied',errorNumber:-1743}).result.status, 'error');
assert.equal(invoke({kind:'invalid'}, null).result.status, 'error');
console.log('PASS: real JXA script routing, JSON data isolation, all four chooser kinds, multi-file Unicode/newline paths, cancel and error');
