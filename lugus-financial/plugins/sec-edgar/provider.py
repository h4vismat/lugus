"""SEC EDGAR protocol adapter. Decoding is pure; HTTP, clock and sessions are edges."""
import base64
from datetime import date, datetime, timezone
from decimal import Decimal
from email.utils import parsedate_to_datetime
import json
import re
import secrets
import socket
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit, unquote, quote
from urllib.request import Request, HTTPRedirectHandler, build_opener

MAX_DOCUMENT = 10 * 1024 * 1024
MAX_SOURCE = 32 * 1024 * 1024
MAX_LINE = 32 * 1024 * 1024

class ProviderError(Exception):
    def __init__(self, kind, message, code=-32000, retry_after_seconds=None):
        super().__init__(message)
        self.kind, self.code, self.retry_after_seconds = kind, code, retry_after_seconds

def invalid(message):
    return ProviderError('invalid_request', message, -32602)

def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds').replace('+00:00', 'Z')

def parse_json(raw):
    def reject(value): raise ValueError('nonfinite number')
    try:
        return json.loads(raw, parse_float=Decimal, parse_constant=reject)
    except (ValueError, UnicodeError, RecursionError) as exc:
        raise ProviderError('malformed_data', 'Invalid source JSON') from exc

def day(value):
    if not isinstance(value, str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}', value):
        raise ValueError('Invalid date')
    date.fromisoformat(value)
    return value

def text(value):
    if not isinstance(value, str) or not value: raise ValueError('Expected nonempty string')
    return value

def query_params(params):
    try:
        company=params['company']
        if company['namespace'] != 'sec:cik': raise ValueError('Expected sec:cik')
        cik=company['value']
        if not isinstance(cik,str) or not re.fullmatch(r'[0-9]{10}',cik) or int(cik)==0: raise ValueError('CIK must contain exactly ten ASCII digits and be nonzero')
        start, end=day(params['filed_from']),day(params['filed_to'])
        if start>end: raise ValueError('Reversed filing dates')
        forms=params.get('forms',[])
        if not isinstance(forms,list) or len(forms)>100 or any(not isinstance(f,str) or not f or len(f)>32 for f in forms): raise ValueError('Invalid forms')
        size=params.get('page_size',100)
        if type(size) is not int or not 1<=size<=1000: raise ValueError('Invalid page size')
        cursor=params.get('cursor')
        if cursor is not None and (not isinstance(cursor,str) or len(cursor)>128): raise ValueError('Invalid cursor')
        return dict(company={'namespace':'sec:cik','value':cik},filed_from=start,filed_to=end,forms=sorted(set(forms)),page_size=size,cursor=cursor)
    except (KeyError,TypeError,ValueError) as exc: raise invalid(str(exc)) from exc

def selected(filed, form, query):
    return query['filed_from']<=filed<=query['filed_to'] and (not query['forms'] or form in query['forms'])

def decode_facts(payload,query,url,retrieved_at):
    try:
        if int(payload['cik']) != int(query['company']['value']): raise ValueError('Company mismatch')
        result=[]
        for namespace,concepts in payload['facts'].items():
            text(namespace)
            for concept, metadata in concepts.items():
                text(concept)
                label=metadata.get('label')
                if label is not None: text(label)
                for unit, observations in metadata['units'].items():
                    text(unit)
                    if not isinstance(observations,list): raise ValueError('Invalid observations')
                    for obs in observations:
                        filed,form=day(obs['filed']),text(obs['form'])
                        value=obs['val']
                        if type(value) not in (int,Decimal): raise ValueError('Invalid decimal')
                        value=Decimal(value)
                        if not value.is_finite() or abs(value.adjusted())>10000: raise ValueError('Invalid decimal size')
                        end=day(obs['end'])
                        period={'kind':'instant','date':end}
                        if 'start' in obs:
                            start=day(obs['start'])
                            if start>end: raise ValueError('Reversed period')
                            period={'kind':'duration','start':start,'end':end}
                        accession=text(obs['accn'])
                        if not re.fullmatch(r'\d{10}-\d{2}-\d{6}',accession): raise ValueError('Invalid accession')
                        fy,fp=obs.get('fy'),obs.get('fp')
                        if fy is not None and type(fy) is not int: raise ValueError('Invalid fiscal year')
                        if fp is not None: text(fp)
                        if selected(filed,form,query):
                            result.append(dict(company=dict(query['company']),namespace=namespace,concept=concept,label=label,value=format(value,'f'),unit=unit,period=period,filing_id=accession,form=form,filed=filed,fiscal_year=fy,fiscal_period=fp,source_url=url,retrieved_at=retrieved_at))
        return result
    except (KeyError,TypeError,AttributeError,ValueError,ArithmeticError) as exc:
        raise ProviderError('malformed_data','Invalid Company Facts payload') from exc

def decode_filings(payload,query,retrieved_at):
    try:
        required=('accessionNumber','filingDate','form')
        count=len(payload['accessionNumber'])
        for key in required:
            if not isinstance(payload[key],list) or len(payload[key])!=count: raise ValueError('Mismatched filing columns')
        for key in ('reportDate','acceptanceDateTime','primaryDocument'):
            if key in payload and (not isinstance(payload[key],list) or len(payload[key])!=count): raise ValueError('Mismatched optional column')
        result=[]
        for i in range(count):
            filed,form=day(payload['filingDate'][i]),text(payload['form'][i])
            accession=text(payload['accessionNumber'][i])
            if not re.fullmatch(r'\d{10}-\d{2}-\d{6}',accession): raise ValueError('Invalid accession')
            def optional(key): return payload[key][i] or None if key in payload else None
            report=optional('reportDate')
            if report is not None: day(report)
            accepted=optional('acceptanceDateTime')
            if accepted:
                parsed=datetime.fromisoformat(accepted.replace('Z','+00:00'))
                accepted=parsed.astimezone(timezone.utc).isoformat().replace('+00:00','Z') if parsed.tzinfo else None
            document=optional('primaryDocument')
            if document:
                if not isinstance(document,str): raise ValueError('Invalid primary document')
                decoded=unquote(document)
                if '\\' in decoded or any(ord(c)<32 for c in decoded) or any(part in ('','.','..') for part in decoded.split('/')):
                    raise ValueError('Invalid primary document')
            url=f"https://www.sec.gov/Archives/edgar/data/{int(query['company']['value'])}/{accession.replace('-','')}/"
            if document: url+=quote(document,safe='/-_.')
            if selected(filed,form,query): result.append(dict(company=dict(query['company']),filing_id=accession,form=form,filed=filed,report_date=report,accepted_at=accepted,primary_document=document,source_url=url,retrieved_at=retrieved_at))
        return result
    except (KeyError,TypeError,ValueError,AttributeError) as exc:
        raise ProviderError('malformed_data','Invalid submissions payload') from exc

def validate_url(url,archive=False):
    try:
        parsed=urlsplit(url)
        path=unquote(parsed.path)
        allowed=parsed.hostname=='www.sec.gov' if archive else parsed.hostname in ('www.sec.gov','data.sec.gov')
        if (parsed.scheme!='https' or not allowed or parsed.port not in (None,443) or parsed.username or parsed.password or parsed.query or parsed.fragment or '\\' in path or any(part in ('.','..') for part in path.split('/')) or any(ord(c)<33 for c in url) or (archive and not path.startswith('/Archives/edgar/data/'))):
            raise ValueError('URL not allowed')
    except (TypeError,ValueError,AttributeError) as exc: raise invalid('Expected an HTTPS SEC archive URL' if archive else 'Invalid SEC URL') from exc
    return url

class SafeRedirect(HTTPRedirectHandler):
    def __init__(self,archive,before_request=lambda:None):
        self.archive=archive; self.before_request=before_request
    def redirect_request(self,req,fp,code,msg,headers,newurl):
        validate_url(newurl,self.archive)
        self.before_request()
        return super().redirect_request(req,fp,code,msg,headers,newurl)

class HttpTransport:
    """Shared per-process limiter: at most two starts/second, three attempts."""
    def __init__(self,user_agent,opener_factory=None,sleep=time.sleep,monotonic=time.monotonic):
        self.user_agent=user_agent
        self.opener_factory=opener_factory or (lambda archive:build_opener(SafeRedirect(archive,self.wait_turn)))
        self.sleep,self.monotonic=sleep,monotonic
        self.next_request=0
    def wait_turn(self):
        delay=max(0,self.next_request-self.monotonic())
        if delay: self.sleep(delay)
        self.next_request=self.monotonic()+0.5
    def get(self,url,max_bytes,archive=False):
        validate_url(url,archive)
        deadline=self.monotonic()+40
        for attempt in range(3):
            self.wait_turn()
            try:
                request=Request(url,headers={'User-Agent':self.user_agent,'Accept-Encoding':'identity'})
                with self.opener_factory(archive).open(request,timeout=min(10,max(0.1,deadline-self.monotonic()))) as response:
                    validate_url(response.geturl(),archive)
                    length=response.headers.get('Content-Length')
                    if length and int(length)>max_bytes: raise ProviderError('malformed_data','Source exceeds byte limit')
                    chunks=[]; size=0
                    while True:
                        if self.monotonic()>deadline: raise ProviderError('timeout','Source deadline exceeded')
                        chunk=response.read(min(65536,max_bytes+1-size))
                        if not chunk: break
                        size+=len(chunk)
                        if size>max_bytes: raise ProviderError('malformed_data','Source exceeds byte limit')
                        chunks.append(chunk)
                    if length and size!=int(length): raise ProviderError('malformed_data','Truncated source response')
                    return b''.join(chunks),response.headers.get_content_type()
            except HTTPError as exc:
                retry=None
                header=exc.headers.get('Retry-After')
                if header:
                    try: retry=max(0,int(header))
                    except ValueError:
                        try: retry=max(0,int((parsedate_to_datetime(header)-datetime.now(timezone.utc)).total_seconds())+1)
                        except (ValueError,TypeError,OverflowError): pass
                kind='rate_limited' if exc.code==429 else 'not_found' if exc.code==404 else 'unavailable'
                error=ProviderError(kind,f'SEC HTTP {exc.code}',retry_after_seconds=retry)
                transient=exc.code in (429,500,502,503,504)
                exc.close()
            except (TimeoutError,socket.timeout) as exc:
                error=ProviderError('timeout','SEC request timed out'); transient=True; retry=None
            except (URLError,ConnectionError,OSError) as exc:
                kind='timeout' if isinstance(getattr(exc,'reason',None),TimeoutError) else 'unavailable'
                error=ProviderError(kind,'SEC request timed out' if kind=='timeout' else 'SEC request unavailable'); transient=True; retry=None
            except ValueError as exc:
                raise ProviderError('malformed_data','Invalid HTTP metadata') from exc
            delay=retry if retry is not None else 2**attempt
            if not transient or attempt==2 or delay>10 or self.monotonic()+delay>=deadline: raise error
            self.sleep(delay)
        raise error

class Provider:
    def __init__(self,transport=None,clock=utc_now):
        self.transport=transport; self.injected_transport=transport is not None
        self.clock=clock; self.initialized=False; self.sessions={}
    def call(self,method,params):
        if not isinstance(params,dict): raise invalid('Params must be an object')
        if method=='initialize':
            if params.get('protocol_version')!=1 or type(params.get('protocol_version')) is not int: raise ProviderError('unsupported','Protocol version 1 required')
            config=params.get('config',{})
            agent=config.get('user_agent') if isinstance(config,dict) else None
            if not isinstance(agent,str) or not 5<len(agent)<512 or not re.search(r'[^\s@]+@[^\s@]+\.[^\s@]+',agent) or any(ord(c)<32 or ord(c)>126 for c in agent): raise ProviderError('configuration','An identifying User-Agent with contact email is required')
            if not self.injected_transport:
                if self.transport is None: self.transport=HttpTransport(agent)
                else: self.transport.user_agent=agent
            self.sessions.clear(); self.initialized=True
            return {'protocol_version':1,'plugin_id':'sec-edgar','plugin_version':'0.1.0','capabilities':{'filings':1,'fundamentals':1}}
        if method not in ('filings.list','fundamentals.facts','filings.document'): raise ProviderError('unsupported','Method not found',-32601)
        if not self.initialized: raise ProviderError('configuration','Initialize first')
        if method=='filings.document':
            url=validate_url(params.get('source_url'),True)
            maximum=params.get('max_bytes',MAX_DOCUMENT)
            if type(maximum) is not int or not 1<=maximum<=MAX_DOCUMENT: raise invalid('Invalid document byte limit')
            content,media=self.transport.get(url,maximum,archive=True)
            if len(content)>maximum: raise ProviderError('malformed_data','Document exceeds byte limit')
            return dict(source_url=url,media_type=media,content_base64=base64.b64encode(content).decode('ascii'),retrieved_at=self.clock())
        query=query_params(params)
        key=json.dumps([method,{k:v for k,v in query.items() if k!='cursor'}],sort_keys=True)
        cursor=query['cursor']
        if cursor is not None:
            if cursor not in self.sessions or self.sessions[cursor][0]!=key: raise invalid('Cursor is invalid for this query or process')
            _,items,offset=self.sessions[cursor]
        else:
            # One snapshot at a time bounds retained memory. Old cursors expire explicitly.
            self.sessions.clear()
            cik=query['company']['value']; stamp=self.clock()
            if method=='fundamentals.facts':
                url=f'https://data.sec.gov/api/xbrl/companyfacts/CIK{cik}.json'
                raw,_=self.transport.get(url,MAX_SOURCE)
                items=decode_facts(parse_json(raw),query,url,stamp)
            else:
                url=f'https://data.sec.gov/submissions/CIK{cik}.json'
                raw,_=self.transport.get(url,MAX_SOURCE)
                payload=parse_json(raw)
                try:
                    if int(payload['cik'])!=int(cik): raise ValueError('Company mismatch')
                    items=decode_filings(payload['filings']['recent'],query,stamp)
                    files=payload['filings']['files']
                    if not isinstance(files,list) or len(files)>1000: raise ValueError('Invalid historical file list')
                    total=len(raw)
                    for file in files:
                        start,end=day(file['filingFrom']),day(file['filingTo'])
                        if start>end: raise ValueError('Reversed historical filing dates')
                        if start>query['filed_to'] or end<query['filed_from']: continue
                        name=file['name']
                        if not isinstance(name,str) or not re.fullmatch(r'CIK'+cik+r'-submissions-\d+\.json',name): raise ValueError('Invalid history name')
                        raw,_=self.transport.get('https://data.sec.gov/submissions/'+name,MAX_SOURCE-total)
                        total+=len(raw)
                        if total>MAX_SOURCE: raise ProviderError('malformed_data','Submissions exceed session size limit')
                        items.extend(decode_filings(parse_json(raw),query,stamp))
                except (KeyError,TypeError,ValueError) as exc: raise ProviderError('malformed_data','Invalid submissions index') from exc
            offset=0
        end=offset+query['page_size']; next_cursor=None
        if end<len(items):
            next_cursor=secrets.token_urlsafe(24)
            self.sessions[next_cursor]=(key,items,end)
        if cursor is not None: self.sessions.pop(cursor)
        return {'items':items[offset:end],'next_cursor':next_cursor}

def handle(provider,request):
    request_id=None
    try:
        if not isinstance(request,dict) or request.get('jsonrpc')!='2.0' or type(request.get('id')) is not int or not isinstance(request.get('method'),str):
            raise ProviderError('invalid_request','Expected a JSON-RPC request with integer ID',-32600)
        request_id=request['id']
        result=provider.call(request['method'],request.get('params',{}))
        return {'jsonrpc':'2.0','id':request_id,'result':result}
    except ProviderError as exc:
        data={'kind':exc.kind}
        if exc.retry_after_seconds is not None: data['retry_after_seconds']=exc.retry_after_seconds
        return {'jsonrpc':'2.0','id':request_id,'error':{'code':exc.code,'message':str(exc),'data':data}}
