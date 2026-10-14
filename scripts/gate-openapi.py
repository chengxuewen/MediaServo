#!/usr/bin/env python3
"""Validate docs/openapi.yaml (assertions verbatim-synced with the ci.yml openapi-validate step)."""
import yaml
spec = yaml.safe_load(open('docs/openapi.yaml', encoding='utf-8'))
assert spec.get('openapi') == '3.0.3', 'not OpenAPI 3.0.3'
assert spec.get('info', {}).get('title'), 'missing info.title'
assert spec.get('info', {}).get('version'), 'missing info.version'
print(f"openapi ok: {len(spec.get('paths', {}))} paths")
