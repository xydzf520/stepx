#!/usr/bin/env python3
"""Run with --browser /path/to/chrome to include real headless interaction tests."""
import argparse
import contextlib
import io
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('studio_lab_runtime', Path(__file__).with_name('studio-lab-runtime.py'))
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)

HTML = '''<!DOCTYPE html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>body{font:18px sans-serif;margin:20px}button,input{font:inherit;max-width:100%;box-sizing:border-box}button{padding:20px}</style></head><body><h1>任务清单验收</h1><input id="task"><button id="add">添加任务</button><ul id="items"></ul><p id="total">0 项</p><script>document.querySelector('#add').onclick=()=>{const input=document.querySelector('#task');if(!input.value)return;const li=document.createElement('li');li.textContent=input.value;document.querySelector('#items').append(li);localStorage.setItem('last',input.value);document.querySelector('#total').textContent=document.querySelectorAll('li').length+' 项';input.value=''};</script></body></html>'''
CHECKS = [
    {'action':'type','selector':'#task','value':'准备周报','label':'键入任务'},
    {'action':'click','selector':'#add','label':'添加任务'},
    {'action':'assert_text','selector':'#items','expected':'准备周报','label':'任务已显示'},
    {'action':'assert_text','selector':'#total','expected':'1 项','label':'数量已更新'},
    {'action':'assert_value','selector':'#task','expected':'','label':'输入框已清空'},
    {'action':'assert_storage','value':'last','expected':'准备周报','label':'结果已保存'},
]

class StructureTests(unittest.TestCase):
    def test_phone_model_request_uses_bidirectional_channel(self):
        stream=io.StringIO()
        with patch('sys.stdin',io.StringIO(json.dumps({'text':'source code from phone'})+'\n')), contextlib.redirect_stdout(stream):
            text,usage=lab.complete({'protocol':'phone'},'format instructions','user request')
        self.assertEqual(text,'source code from phone')
        self.assertEqual(usage,{})
        self.assertEqual(json.loads(stream.getvalue()),{'event':'model_request','system':'format instructions','prompt':'user request'})

    def test_phone_model_empty_reply_is_failure(self):
        with patch('sys.stdin',io.StringIO('{"text":""}\n')),contextlib.redirect_stdout(io.StringIO()),self.assertRaises(lab.Failure):
            lab.complete({'protocol':'phone'},'rules','task')

    def test_review_uses_only_requirements_and_contract_not_implementation(self):
        bad=[dict(c) for c in CHECKS];bad[3]['expected']='99 项'
        response=json.dumps({'checks':CHECKS,'notes':['添加一项后的数量应为 1。']})
        with patch.object(lab,'complete',return_value=(response,{})) as model:
            reviewed,record=lab.review_checks({'protocol':'phone'},'创建待办清单',bad)
        self.assertEqual(reviewed[3]['expected'],'1 项')
        self.assertEqual(model.call_args.args[0]['_purpose'],'acceptance_review')
        self.assertNotIn('<html',model.call_args.args[2])
        self.assertFalse(record['implementation_visible'])
        with patch.object(lab,'complete',return_value=('looks good',{})),self.assertRaises(lab.Failure):
            lab.review_checks({},'创建待办清单',bad)

    def test_raw_html_and_metadata_blocks_do_not_need_html_json_escaping(self):
        text='```html\n'+HTML+'\n```\n```json\n'+json.dumps(dict(name='轻记',checks=CHECKS))+'\n```'
        program=lab.parse_program(text)
        self.assertEqual(program['html'],HTML)
        self.assertEqual(program['name'],'轻记')
        with self.assertRaises(lab.Failure): lab.parse_program(text+'\nunexpected prose')
    def test_missing_or_invalid_function_checks_require_repair(self):
        for text in [HTML, '```html\n'+HTML+'\n```',
                     '```html\n'+HTML+'\n```\n```json\n'+json.dumps({'name':'test','checks':[]})+'\n```']:
            with self.assertRaises(lab.Failure): lab.parse_program(text)
        roots=[{'action':'key','value':'Tab'},
               {'action':'assert_count','selector':'html','expected':1},
               {'action':'assert_count','selector':'body','expected':1}]
        with self.assertRaises(lab.Failure): lab.validate_checks(roots)

    def test_generated_pass_flag_has_no_effect(self):
        result=lab.parse_program(json.dumps(dict(name='test',html=HTML,checks=CHECKS,passed=True)))
        self.assertNotIn('passed',result)
    def test_assert_value_alias_is_normalized_without_overriding_expected(self):
        checks=[dict(x) for x in CHECKS]+[{'action':'assert_value','selector':'#task','value':'准备周报'}]
        lab.validate_checks(checks)
        self.assertEqual(checks[-1]['expected'],'准备周报')
        checks[-1]['expected']='explicit'
        lab.validate_checks(checks)
        self.assertEqual(checks[-1]['expected'],'explicit')
    def test_no_empty_assertion_or_arbitrary_script(self):
        for bad in [[],[{'action':'eval','value':'alert(1)'}]*3,
                    [{'action':'click','selector':'button'},{'action':'assert_text','selector':'body','expected':''},{'action':'assert_count','selector':'p','expected':1}]]:
            with self.assertRaises(lab.Failure): lab.validate_checks(bad)
    def test_requests_cannot_have_unbounded_waits(self):
        checks=CHECKS+[{'action':'wait','ms':60000}]
        with self.assertRaises(lab.Failure): lab.validate_checks(checks)
    def test_does_not_treat_plaintext_or_incomplete_html_as_app(self):
        for html in ['hello','<h1>hello</h1>']:
            with self.assertRaises(lab.Failure): lab.parse_program(json.dumps(dict(html=html,checks=CHECKS)))

class StreamTests(unittest.TestCase):
    def response(self, protocol, events):
        payload=b''.join(b'data: '+(item.encode() if isinstance(item,str) else json.dumps(item).encode())+b'\n\n' for item in events)
        class Opener:
            def open(self,*args,**kwargs): return io.BytesIO(payload)
        with patch.object(lab.urllib.request,'build_opener',return_value=Opener()), contextlib.redirect_stdout(io.StringIO()):
            return lab.stream_complete({'protocol':protocol,'endpoint':'https://test.invalid/v1','key':'test-only'},'/messages' if protocol=='anthropic' else '/chat/completions',{})
    def test_openai_stream_joins_content_and_usage(self):
        text,usage=self.response('openai',[
            {'choices':[{'delta':{'reasoning_content':'not displayed'},'finish_reason':None}]},
            {'choices':[{'delta':{'content':'hello '},'finish_reason':None}]},
            {'choices':[{'delta':{'content':'world'},'finish_reason':'stop'}]},
            {'choices':[],'usage':{'prompt_tokens':2,'completion_tokens':3}},'[DONE]'])
        self.assertEqual(text,'hello world'); self.assertEqual(usage['completion_tokens'],3)
    def test_provider_credential_echo_is_redacted_before_persistence(self):
        text,_=self.response('openai',[{'choices':[{'delta':{'content':'test-only'},'finish_reason':'stop'}]},'[DONE]'])
        self.assertEqual(text,'[credential-redacted]')
    def test_openai_requires_complete_final_marker(self):
        with self.assertRaises(lab.Failure):
            self.response('openai',[{'choices':[{'delta':{'content':'partial'},'finish_reason':'stop'}]}])
    def test_anthropic_stream_joins_text_and_usage(self):
        text,usage=self.response('anthropic',[
            {'type':'message_start','message':{'usage':{'input_tokens':5}}},
            {'type':'content_block_delta','delta':{'type':'thinking_delta','thinking':'not displayed'}},
            {'type':'content_block_delta','delta':{'type':'text_delta','text':'hello'}},
            {'type':'message_delta','delta':{'stop_reason':'end_turn'},'usage':{'output_tokens':8}},
            {'type':'message_stop'}])
        self.assertEqual(text,'hello');self.assertEqual(usage,{'input_tokens':5,'output_tokens':8})
    def test_truncation_is_never_a_completed_program(self):
        with self.assertRaises(lab.Failure):
            self.response('openai',[{'choices':[{'delta':{'content':'partial'},'finish_reason':'length'}]},'[DONE]'])

class BrowserTests(unittest.TestCase):
    browser = None
    def setUp(self):
        if not self.browser: self.skipTest('--browser not supplied')
        self.directory=tempfile.TemporaryDirectory(prefix='stepx-lab-test-')
        self.root=Path(self.directory.name)
    def tearDown(self):
        if hasattr(self,'directory'): self.directory.cleanup()
    def test_real_input_click_and_state(self):
        result=lab.verify(self.browser,dict(html=HTML,checks=CHECKS),self.root)
        self.assertTrue(result['passed'],result)
        self.assertTrue((self.root/'preview.png').is_file())
        self.assertEqual(result['html_sha256'],hashlib.sha256(HTML.encode()).hexdigest())
    def test_assert_value_alias_verifies_actual_input(self):
        checks=[dict(CHECKS[0]),{'action':'assert_value','selector':'#task','value':'准备周报'},*CHECKS[1:]]
        result=lab.verify(self.browser,dict(html=HTML,checks=checks),self.root)
        self.assertTrue(result['passed'],result)
    def test_wrong_assertion_and_js_error_are_failed(self):
        checks=[dict(x) for x in CHECKS]
        checks[3]['expected']='99 项'
        html=HTML.replace('</body>','<script>setTimeout(()=>{throw Error("expected smoke error")},50)</script></body>')
        result=lab.verify(self.browser,dict(html=html,checks=checks),self.root)
        self.assertFalse(result['passed'])
        failures=[x['label'] for x in result['checks'] if not x['passed']]
        self.assertIn('数量已更新',failures)
        self.assertIn('JavaScript 运行错误',failures)
    def test_repairs_from_browser_failure_and_keeps_both_versions(self):
        # The model is simulated in this regression; the browser verifier is real.
        calls=[]
        original=lab.complete
        def fake_model(config, system, prompt, small=False):
            if config.get('_purpose')=='acceptance_review':
                return json.dumps({'checks':CHECKS,'notes':[]}),{}
            calls.append(prompt)
            html=HTML if len(calls)>1 else HTML.replace("document.querySelector('#items').append(li);", "")
            checks=[dict(c) for c in CHECKS]
            if len(calls)>1: checks[3]['expected']='99 项'
            return json.dumps(dict(name='修复验收',html=html,checks=checks)), {}
        lab.complete=fake_model
        output=io.StringIO()
        try:
            with contextlib.redirect_stdout(output):
                lab.generate(dict(root=str(self.root),config={},prompt='创建能添加任务的任务清单',browser=self.browser))
        finally: lab.complete=original
        events=[json.loads(line) for line in output.getvalue().splitlines()]
        artifacts=[event['artifact'] for event in events if event['event']=='artifact']
        self.assertEqual([a['passed'] for a in artifacts],[False,True])
        self.assertEqual(events[-1]['event'],'complete')
        self.assertIn('Actual independent browser feedback',calls[1])
        saved=json.loads((self.root/'round-2/checks.json').read_text())
        self.assertEqual(saved[3]['expected'],'1 项')
        self.assertTrue((self.root/'round-1/index.html').is_file())
        self.assertTrue((self.root/'round-2/index.html').is_file())
    def test_external_script_is_blocked_and_reported(self):
        html=HTML.replace('</head>','<script src="https://example.invalid/cannot-load.js"></script></head>')
        result=lab.verify(self.browser,dict(html=html,checks=CHECKS),self.root)
        self.assertFalse(result['passed'])
        self.assertFalse(next(x for x in result['checks'] if x['label']=='离线运行')['passed'])

    def test_wrong_model_expectation_is_reviewed_before_repairing_correct_code(self):
        wrong=[dict(c) for c in CHECKS];wrong[3]['expected']='99 项'
        def model(config, system, prompt, small=False):
            if config.get('_purpose')=='acceptance_review':
                return json.dumps({'checks':CHECKS,'notes':['一次添加应为 1 项，不能为 99 项。']}),{}
            return json.dumps(dict(name='待办清单',html=HTML,checks=wrong)),{}
        output=io.StringIO()
        with patch.object(lab,'complete',model),contextlib.redirect_stdout(output):
            lab.generate(dict(root=str(self.root),config={},prompt='制作待办清单',browser=self.browser))
        events=[json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(events[-1]['event'],'complete')
        self.assertFalse((self.root/'round-2').exists())
        self.assertEqual((self.root/'round-1/index.html').read_text(),HTML)
        report=json.loads((self.root/'round-1/report.json').read_text())
        self.assertTrue(report['passed'])
        self.assertTrue(report['acceptance_review']['notes'])

    def test_unit_oracle_rejects_reversed_conversion_despite_model_expectations(self):
        checks=lab.deterministic_checks('制作厘米和英寸互转工具')
        html='''<!DOCTYPE html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><h1>单位换算验收</h1><label>厘米<input data-unit="cm"></label><label>英寸<input data-unit="in"></label><p data-conversion-feedback>请输入数值</p><script>const cm=document.querySelector('[data-unit="cm"]'),inch=document.querySelector('[data-unit="in"]'),tip=document.querySelector('p');for(const [a,b,f] of [[cm,inch,1/2.54],[inch,cm,2.54]])a.oninput=()=>{const n=Number(a.value);if(!a.value.trim()||!Number.isFinite(n)){b.value='';tip.textContent='请输入有效数字';return}b.value=String(n*f);tip.textContent='换算完成'};</script></body></html>'''
        result=lab.verify(self.browser,dict(html=html,checks=checks),self.root)
        self.assertTrue(result['passed'],result)
        broken=html.replace('[inch,cm,2.54]','[inch,cm,1/2.54]')
        broken_root=self.root/'broken';broken_root.mkdir()
        result=lab.verify(self.browser,dict(html=broken,checks=checks),broken_root)
        self.assertFalse(result['passed'])
        self.assertTrue(any('6.4516' in c['label'] and not c['passed'] for c in result['checks']))

    def test_text_oracle_and_browser_input_keep_replacement_and_insertion_distinct(self):
        html='''<!DOCTYPE html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><h1>文本统计验收</h1><textarea id="text" data-text-stat-input></textarea><span data-stat="characters">0</span><span data-stat="lines">0</span><script>const t=document.querySelector('textarea');t.oninput=()=>{document.querySelector('[data-stat="characters"]').textContent=Array.from(t.value).length;document.querySelector('[data-stat="lines"]').textContent=t.value?t.value.split('\\n').length:0};</script></body></html>'''
        checks=lab.deterministic_checks('制作文本统计工具，计算字符数和行数')
        checks += [dict(action='type',selector='#text',value='hello'),dict(action='key',selector='#text',value='Enter'),dict(action='insert',selector='#text',value='中文'),dict(action='assert_value',selector='#text',expected='hello\n中文')]
        result=lab.verify(self.browser,dict(html=html,checks=checks),self.root)
        self.assertTrue(result['passed'],result)
    def test_reload_retains_saved_tasks(self):
        restore="const saved=localStorage.getItem('last');if(saved){const li=document.createElement('li');li.textContent=saved;document.querySelector('#items').append(li);document.querySelector('#total').textContent='1 项'};"
        html=HTML.replace('<script>','<script>'+restore)
        checks=CHECKS+[{'action':'reload'},{'action':'assert_text','selector':'#items','expected':'准备周报'},{'action':'assert_text','selector':'#total','expected':'1 项'}]
        result=lab.verify(self.browser,dict(html=html,checks=checks),self.root)
        self.assertTrue(result['passed'],result)
    def test_preview_does_not_contain_acceptance_records(self):
        observed=[]
        original=lab.Browser.call
        def capture(browser,method,*args,**kwargs):
            if method=='Page.captureScreenshot':
                observed.append(browser.evaluate('({items:document.querySelectorAll("#items li").length,stored:localStorage.getItem("last")})'))
            return original(browser,method,*args,**kwargs)
        with patch.object(lab.Browser,'call',capture):
            result=lab.verify(self.browser,dict(html=HTML,checks=CHECKS),self.root)
        self.assertTrue(result['passed'],result)
        self.assertEqual(observed,[{'items':0,'stored':None}])
    def test_mobile_viewport_is_checked(self):
        html=HTML.replace('<meta name="viewport" content="width=device-width,initial-scale=1">','')
        result=lab.verify(self.browser,dict(html=html,checks=CHECKS),self.root)
        self.assertFalse(next(x for x in result['checks'] if x['label']=='手机布局')['passed'])

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--browser')
    args,rest=parser.parse_known_args();BrowserTests.browser=args.browser
    unittest.main(argv=[__file__,*rest])
