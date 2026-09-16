"""SEC identity evidence: pure parsing/matching with bounded snapshot sessions."""
import hashlib
import json
import re
import secrets
import unicodedata
from provider import MAX_SOURCE, ProviderError, invalid, parse_json

DIRECTORY_URL = 'https://www.sec.gov/files/company_tickers_exchange.json'
DIRECTORY_COVERAGE = 'SEC company ticker/exchange directory snapshot; not all SEC filers'

def bounded_text(value, maximum):
    if not isinstance(value, str) or not value.strip() or len(value.encode('utf-8')) > maximum:
        raise ValueError('Expected bounded nonempty text')
    if any(unicodedata.category(char) == 'Cc' for char in value):
        raise ValueError('Control characters are not allowed')
    return value

def cik(value):
    if not isinstance(value, str) or not re.fullmatch(r'[0-9]{1,10}', value) or int(value) == 0:
        raise ValueError('Expected a positive CIK of at most ten ASCII digits')
    return value.zfill(10)

def source_cik(value):
    if type(value) is int:
        value = str(value)
    return cik(value)

def object_fields(value, allowed):
    if not isinstance(value, dict) or set(value) - set(allowed):
        raise ValueError('Unexpected object fields')

def identifier(value):
    object_fields(value, ('namespace', 'value'))
    return {'namespace': bounded_text(value['namespace'], 128),
            'value': bounded_text(value['value'], 128)}

def normalized_name(value):
    return ' '.join(value.split()).lower()

def normalized_ticker(value):
    return value.strip().translate(str.maketrans('abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'))

def search_params(params):
    try:
        object_fields(params, ('query', 'page_size', 'cursor'))
        query = params['query']
        if query['kind'] == 'name':
            object_fields(query, ('kind', 'text'))
            parsed = {'kind': 'name', 'text': bounded_text(query['text'], 256)}
        elif query['kind'] == 'identifier':
            object_fields(query, ('kind', 'identifier', 'exchange'))
            entity = identifier(query['identifier'])
            if entity['namespace'] not in ('sec:cik', 'sec:ticker'):
                raise ProviderError('unsupported', 'Unsupported search identifier namespace')
            if entity['namespace'] == 'sec:cik':
                entity['value'] = cik(entity['value'])
            exchange = query.get('exchange')
            if exchange is not None:
                exchange = identifier(exchange)
                if exchange['namespace'] != 'sec:exchange' or entity['namespace'] != 'sec:ticker':
                    raise ProviderError('unsupported', 'Exchange qualifiers require sec:ticker and sec:exchange')
            parsed = {'kind': 'identifier', 'identifier': entity, 'exchange': exchange}
        else:
            raise ValueError('Unknown query kind')
        size = params.get('page_size', 100)
        if type(size) is not int or not 1 <= size <= 100:
            raise ValueError('Page size must be 1..100')
        cursor = params.get('cursor')
        if cursor is not None:
            bounded_text(cursor, 128)
        return parsed, size, cursor
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise invalid(str(exc)) from exc

def lookup_params(params):
    try:
        object_fields(params, ('identifier',))
        entity = identifier(params['identifier'])
        if entity['namespace'] != 'sec:cik':
            raise ProviderError('unsupported', 'Lookup requires sec:cik; search listing identifiers')
        return cik(entity['value'])
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise invalid(str(exc)) from exc

def listing(ticker, exchange):
    ticker = bounded_text(ticker, 128)
    if exchange is not None:
        exchange = {'namespace': 'sec:exchange', 'value': bounded_text(exchange, 128)}
    return {'ticker': {'namespace': 'sec:ticker', 'value': ticker}, 'exchange': exchange}

def candidate(cik_value, name, listings, url, checksum, stamp, unlimited_research=False):
    unique = {json.dumps(item, sort_keys=True): item for item in listings}
    if not unlimited_research and len(unique) > 1000:
        raise ValueError('Too many listing associations')
    return {'identifier': {'namespace': 'sec:cik', 'value': cik_value},
            'name': bounded_text(name, 1024), 'aliases': [],
            'listings': [unique[key] for key in sorted(unique)],
            'source_url': url, 'source_checksum': checksum, 'retrieved_at': stamp,
            'match_reasons': []}

def decode_directory(payload, checksum, stamp, unlimited_research=False):
    try:
        fields, rows = payload['fields'], payload['data']
        if not isinstance(fields, list) or any(not isinstance(field, str) for field in fields) or len(set(fields)) != len(fields):
            raise ValueError('Invalid or duplicate directory fields')
        if not all(field in fields for field in ('cik', 'name', 'ticker', 'exchange')) or not isinstance(rows, list):
            raise ValueError('Missing directory fields or rows')
        grouped = {}
        for values in rows:
            if not isinstance(values, list) or len(values) != len(fields):
                raise ValueError('Malformed directory row')
            row = dict(zip(fields, values))
            key = source_cik(row['cik'])
            name = bounded_text(row['name'], 1024)
            association = listing(row['ticker'], row['exchange'])
            if key not in grouped:
                grouped[key] = [name, []]
            if grouped[key][0] != name:
                raise ValueError('Inconsistent names for the same CIK in directory snapshot')
            grouped[key][1].append(association)
        return [candidate(key, name, listings, DIRECTORY_URL, checksum, stamp, unlimited_research=unlimited_research)
                for key, (name, listings) in grouped.items()]
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise ProviderError('malformed_data', 'Invalid SEC company directory') from exc

def decode_lookup(payload, expected, url, checksum, stamp, unlimited_research=False):
    try:
        if source_cik(payload['cik']) != expected:
            raise ValueError('Lookup CIK mismatch')
        tickers, exchanges = payload['tickers'], payload['exchanges']
        if not isinstance(tickers, list) or not isinstance(exchanges, list) or len(tickers) != len(exchanges):
            raise ValueError('Mismatched listing arrays')
        return candidate(expected, payload['name'], [listing(t, e) for t, e in zip(tickers, exchanges)], url, checksum, stamp, unlimited_research=unlimited_research)
    except (KeyError, TypeError, ValueError, AttributeError) as exc:
        raise ProviderError('malformed_data', 'Invalid SEC lookup payload') from exc

def match_candidates(items, query):
    result = []
    for item in items:
        reason = None
        if query['kind'] == 'name':
            name, text = normalized_name(item['name']), normalized_name(query['text'])
            if name == text:
                reason = 'exact_name'
            elif text in name:
                reason = 'name_substring'
        else:
            ticker, exchange = query['identifier']['value'], query['exchange']
            if any(normalized_ticker(entry['ticker']['value']) == normalized_ticker(ticker)
                   and (exchange is None or entry['exchange'] is not None and
                        normalized_ticker(entry['exchange']['value']) == normalized_ticker(exchange['value']))
                   for entry in item['listings']):
                reason = 'exact_identifier'
        if reason:
            result.append(dict(item, match_reasons=[reason]))
    return sorted(result, key=lambda item: (item['match_reasons'] == ['name_substring'], normalized_name(item['name']), item['identifier']['value']))

class Resolution:
    def __init__(self, transport, clock, unlimited_research=False):
        self.unlimited_research = unlimited_research
        self.transport, self.clock = transport, clock
        self.sessions = {}

    def fetch(self, url):
        raw, _ = self.transport.get(url, None if self.unlimited_research else MAX_SOURCE)
        if not self.unlimited_research and len(raw) > MAX_SOURCE:
            raise ProviderError('malformed_data', 'Source exceeds byte limit')
        return parse_json(raw), hashlib.sha256(raw).hexdigest(), self.clock()

    def lookup(self, params):
        key = lookup_params(params)
        url = f'https://data.sec.gov/submissions/CIK{key}.json'
        payload, checksum, stamp = self.fetch(url)
        return decode_lookup(payload, key, url, checksum, stamp, unlimited_research=self.unlimited_research)

    def search(self, params):
        query, size, cursor = search_params(params)
        key = json.dumps([query, size], sort_keys=True)
        if cursor is not None:
            session = self.sessions.get(cursor)
            if session is None or session[0] != key:
                raise invalid('Cursor is invalid for this query or process')
            _, items, offset, snapshot, coverage = session
        else:
            self.sessions.clear()
            snapshot = secrets.token_urlsafe(24)
            if query['kind'] == 'identifier' and query['identifier']['namespace'] == 'sec:cik':
                coverage = 'SEC submissions lookup for CIK ' + query['identifier']['value']
                try:
                    item = self.lookup({'identifier': query['identifier']})
                    items = [dict(item, match_reasons=['exact_identifier'])]
                except ProviderError as exc:
                    if exc.kind != 'not_found':
                        raise
                    items = []
            else:
                payload, checksum, stamp = self.fetch(DIRECTORY_URL)
                items = match_candidates(decode_directory(payload, checksum, stamp, unlimited_research=self.unlimited_research), query)
                coverage = DIRECTORY_COVERAGE
            offset = 0
        end, next_cursor = offset + size, None
        if end < len(items):
            next_cursor = secrets.token_urlsafe(24)
            self.sessions[next_cursor] = (key, items, end, snapshot, coverage)
        if cursor is not None:
            del self.sessions[cursor]
        return {'items': items[offset:end], 'next_cursor': next_cursor,
                'snapshot': snapshot, 'coverage': coverage}
