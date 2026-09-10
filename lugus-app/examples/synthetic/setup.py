"""Write external deterministic CLI inputs into a caller-selected empty directory."""
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
root.mkdir(parents=True, exist_ok=True)
if any(root.iterdir()):
    raise SystemExit("choose an empty acceptance directory")
fixture = pathlib.Path(__file__).resolve().parents[2] / "tests/fixtures/worker.py"

def write(name, value):
    (root / name).write_text(json.dumps(value, indent=2) + "\n")

write("manifest.json", {
    "id": "worker-fixture", "version": "1", "protocol_version": 1,
    "command": "python3", "args": [str(fixture)],
})
limits = {
    "queue_capacity": 4, "max_concurrent_jobs": 2,
    "operation_timeout": {"secs": 10, "nanos": 0},
    "max_pages_per_fetch": 3, "max_items_per_fetch": 100,
    "max_bytes_per_fetch": 1048576, "max_document_bytes": 32768,
    "max_input_bytes": 65536, "max_output_bytes": 32768,
    "max_read_page_items": 20, "max_read_page_bytes": 16384,
}
config = {
    "financial_path": "financial.sqlite", "application_path": "application.sqlite",
    "providers": [{
        "instance_id": name, "manifest": "manifest.json", "active": True,
        "config": {"mode": mode, "barrier": str(root / name)},
    } for name, mode in [("filings", "apple"), ("market", "apple"), ("unavailable", "startup_failure")]],
    "limits": limits,
    "host_bounds": {"max_pending_jobs": 16, "max_terminal_jobs": 16, "event_capacity": 16},
}
write("config.json", config)
write("offline.json", dict(config, providers=[]))
query = {
    "company": {"namespace": "sec:cik", "value": "0000320193"},
    "filed_from": "2024-01-01", "filed_to": "2024-12-31",
    "forms": [], "page_size": 10, "cursor": None,
}
write("filings.json", {"operation": "filings", "instance_id": "filings", "query": query})
write("unavailable.json", {"operation": "filings", "instance_id": "unavailable", "query": query})
write("prices.json", {
    "operation": "prices", "instance_id": "market", "query": {
        "instrument": {"namespace": "fixture:symbol", "value": "EXPLICIT"},
        "start": "2024-01-01", "end": "2024-01-03", "page_size": 10, "cursor": None,
    },
})
(root / "thesis.txt").write_text(
    "Fixture research thesis: assess explicitly selected filing and market evidence "
    "while preserving each provider's exact source and retrieval provenance.\n"
)
print(root)

write("binding.json", {"company_instance_id":"filings","market_instance_id":"market","input":"Apple","native_namespace":"yahoo:symbol","start":"2024-01-01","end":"2024-01-03","page_size":10})
for name, target, mode in [("wrong-issuer.json","market","apple_wrong_issuer"),("wrong-exchange.json","market","apple_wrong_exchange"),("missing-evidence.json","market","apple_missing"),("multiple-listings.json","filings","apple_multiple")]:
    altered = json.loads(json.dumps(config))
    for provider in altered['providers']:
        if provider['instance_id'] == target: provider['config']['mode'] = mode
    write(name, altered)
