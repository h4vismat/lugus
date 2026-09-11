"""Refresh the offline presentation map from the pinned SEC plugin environment."""
import json
from pathlib import Path

root = Path(__file__).resolve().parents[2]
site = next((root / 'lugus-financial/plugins/sec-edgar/.venv/lib').glob('python*/site-packages'))
source = site / 'edgar/xbrl/standardization/gaap_mappings.json'
statements = {'IncomeStatement': 'income', 'BalanceSheet': 'balance', 'CashFlowStatement': 'cash_flow'}
mappings = json.loads(source.read_text())
# Unknown and low-confidence concepts stay accessible under Other Metrics.
selected = {concept: statements[item['statement']] for concept, item in mappings.items()
            if item.get('statement') in statements and item.get('confidence', 0) >= 0.8}
output = root / 'lugus-desktop/src/statement-mappings.ts'
output.write_text('// Generated from EdgarTools 5.57.0 gaap_mappings.json; confidence >= 0.8.\n'
                  '// MIT copyright Dwight Gunning; see public/licenses/edgartools.txt.\n'
                  '// Refresh with scripts/update-statement-mappings.py. Do not edit manually.\n'
                  'export const statementMappings:Readonly<Record<string,string>> = ' +
                  json.dumps(selected, indent=2, sort_keys=True) + ';\n')
license_path = site / 'edgartools-5.57.0.dist-info/licenses/LICENSE.txt'
(root / 'lugus-desktop/public/licenses/edgartools.txt').write_bytes(license_path.read_bytes())
print(f'Exported {len(selected)} concept mappings')
