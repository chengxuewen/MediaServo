#!/usr/bin/env python3
"""openapi.yaml 校验（与 ci.yml openapi-validate 断言逐字同步形）。"""
import yaml
spec = yaml.safe_load(open('docs/openapi.yaml', encoding='utf-8'))
assert spec.get('openapi') == '3.0.3', 'not OpenAPI 3.0.3'
assert spec.get('info', {}).get('title'), 'missing info.title'
assert spec.get('info', {}).get('version'), 'missing info.version'
print(f"openapi ok: {len(spec.get('paths', {}))} paths")
