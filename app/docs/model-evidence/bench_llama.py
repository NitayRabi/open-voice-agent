import json,time,urllib.request,pathlib
out=pathlib.Path(__file__).parent
cases=[('conversation','I had a long day. Suggest one simple way to unwind.'),('delegation','Find the latest release of the Rust programming language and summarize what changed.'),('conversation','Explain why the sky is blue in one short sentence.'),('delegation','Compare the files in my project and identify the biggest performance bottleneck.')]
tool={'type':'function','function':{'name':'delegate_task','description':'Delegate research, coding, calculations, or actions to the capable brain.','parameters':{'type':'object','properties':{'request':{'type':'string'}},'required':['request']}}}
rows=[]
for repeat in range(4):
 for kind,prompt in cases:
  body={'model':'local-conversation','messages':[{'role':'system','content':'You are a natural, concise voice assistant. Answer casual conversation in one short sentence. Use delegate_task for research, coding, calculations, or actions.'},{'role':'user','content':prompt}],'tools':[tool],'temperature':0,'max_tokens':128,'stream':True,'stream_options':{'include_usage':True}}
  start=time.perf_counter();first=None;chunks=[];timings=None;usage=None
  with urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8011/v1/chat/completions',data=json.dumps(body).encode(),headers={'Content-Type':'application/json'}),timeout=120) as r:
   for line in r:
    if not line.startswith(b'data: '):continue
    if line.strip()==b'data: [DONE]':break
    d=json.loads(line[6:]); elapsed=time.perf_counter()-start
    for c in d.get('choices',[]):
     delta=c.get('delta',{})
     if delta.get('content') or delta.get('tool_calls'):
      if first is None:first=elapsed
      chunks.append(delta)
    if d.get('timings'):timings=d['timings']
    if d.get('usage'):usage=d['usage']
  row={'repeat':repeat,'kind':kind,'prompt':prompt,'first_delta_s':first,'complete_s':time.perf_counter()-start,'chunks':chunks,'usage':usage,'timings':timings}
  rows.append(row); print(json.dumps({k:v for k,v in row.items() if k not in ['chunks','prompt']}),flush=True)
  (out/'llama-results.json').write_text(json.dumps(rows,indent=2))
