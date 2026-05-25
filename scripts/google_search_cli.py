#!/usr/bin/env python3

import argparse
import asyncio
import html
import json
import re
import sys
from dataclasses import asdict, dataclass
from typing import Optional
from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse

from bs4 import BeautifulSoup
from curl_cffi.requests import AsyncSession

GSA_USER_AGENT = (
    "Mozilla/5.0 (Linux; Android 12; SM-S901U) "
    "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.88 "
    "Mobile Safari/537.36 NSTNWV"
)

DEFAULT_HEADERS = {
    "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    "Accept-Language": "vi-VN,vi;q=0.9,en;q=0.8",
    "User-Agent": GSA_USER_AGENT,
}

TRACKING_PARAMS = {
    "fbclid",
    "gclid",
    "dclid",
    "gbraid",
    "wbraid",
    "ved",
    "ei",
    "sei",
    "sa",
    "usg",
    "aqs",
    "oq",
    "source",
    "sxsrf",
    "sca_esv",
    "rlz",
    "iflsig",
    "sclient",
    "qh_ss",
    "srsltid",
}


@dataclass
class SearchResult:
    title: str
    url: str
    snippet: str


def build_query_candidates(query: str, query_type: str) -> list[str]:
    if query_type == "phone":
        candidates = [
            query,
            f'"{query}"',
            f"{query} lừa đảo",
            f'"{query}" lừa đảo',
        ]
    elif query_type == "bank":
        candidates = [
            query,
            f'"{query}"',
            f'"{query}" tài khoản ngân hàng lừa đảo',
        ]
    else:
        candidates = [
            query,
            f'"{query}"',
            f'"{query}" cảnh báo lừa đảo',
        ]

    deduped: list[str] = []
    seen: set[str] = set()
    for item in candidates:
        if item not in seen:
            seen.add(item)
            deduped.append(item)
    return deduped


def normalize_google_url(href: str) -> str:
    if href.startswith("/url?q="):
        parsed = urlparse(f"https://www.google.com{href}")
        for key, value in parse_qsl(parsed.query, keep_blank_values=True):
            if key == "q":
                return value
    return href


def canonicalize_result_url(url: str) -> str:
    parsed = urlparse(url)
    if not parsed.scheme or not parsed.netloc:
        return url
    filtered = [
        (key, value)
        for key, value in parse_qsl(parsed.query, keep_blank_values=True)
        if not key.startswith("utm_") and key not in TRACKING_PARAMS
    ]
    return urlunparse(
        (
            parsed.scheme,
            parsed.netloc,
            parsed.path,
            parsed.params,
            urlencode(filtered),
            "",
        )
    )


def parse_google_basic_results(html: str, limit: int) -> list[SearchResult]:
    soup = BeautifulSoup(html, "html.parser")
    seen: set[str] = set()
    results: list[SearchResult] = []
    snippet_selectors = [
        "div.VwiC3b",
        "div[data-sncf='1']",
        "span.aCOpRe",
        "div.s3v9rd",
        "div.yXK7lf",
        "div.BNeawe.s3v9rd",
        "div.uhHOwf.BYbUcd",
    ]

    for heading in soup.select("h3"):
        title = heading.get_text(" ", strip=True)
        if not title:
            continue

        anchor = heading.find_parent("a", href=True)
        if not anchor:
            continue
        url = normalize_google_url(anchor.get("href", ""))
        if not (url.startswith("http://") or url.startswith("https://")):
            continue

        canonical = canonicalize_result_url(url)
        if canonical in seen:
            continue
        seen.add(canonical)

        snippet = ""
        for parent in heading.parents:
            if not getattr(parent, "select", None):
                continue
            for selector in snippet_selectors:
                node = parent.select_one(selector)
                if node:
                    snippet = node.get_text(" ", strip=True)
                    break
            if snippet:
                break

        results.append(SearchResult(title=title, url=url, snippet=snippet))
        if len(results) >= limit:
            break

    return results


def decode_google_escaped_path(value: str) -> str:
    return (
        value.replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("\\u003d", "=")
        .replace("\\u003f", "?")
        .replace("\\u0025", "%")
        .replace("&amp;", "&")
        .rstrip("\\")
    )


def extract_google_basic_results_url(body: str) -> Optional[str]:
    match = re.search(r'(/search\?q=.*?gbv=1.*?sei=[^"\'\\<\s]+)', body)
    if match:
        path = decode_google_escaped_path(match.group(1))
        return f"https://www.google.com{path}"

    fragments: list[str] = []
    decoder = json.JSONDecoder()
    index = 0
    while index < len(body):
        try:
            obj, end = decoder.raw_decode(body, index)
            index = end
            if isinstance(obj, dict) and isinstance(obj.get("d"), str):
                fragments.append(obj["d"])
        except Exception:
            index += 1
    soup = BeautifulSoup("".join(fragments), "html.parser")
    meta = soup.select_one('noscript meta[http-equiv="refresh"], meta[http-equiv="refresh"]')
    if not meta:
        return None
    content = meta.get("content", "")
    if "url=" not in content:
        return None
    path = decode_google_escaped_path(html.unescape(content.split("url=", 1)[1].strip()))
    return f"https://www.google.com{path}"


async def run_google_cli(
    query: str,
    query_type: str,
    proxy: Optional[str],
    limit: int,
) -> dict:
    candidates = build_query_candidates(query, query_type)
    seen: set[str] = set()
    aggregated: list[SearchResult] = []
    successful_queries: list[str] = []
    raw_samples: list[str] = []

    async with AsyncSession() as session:
        for search_query in candidates:
            response = await session.get(
                "https://www.google.com/search",
                params={
                    "q": search_query,
                    "hl": "vi",
                    "gl": "vn",
                    "tch": "1",
                },
                headers=DEFAULT_HEADERS,
                impersonate="chrome136",
                timeout=15,
                proxy=proxy,
            )
            body = response.text
            if len(raw_samples) < 2:
                raw_samples.append(body[:1000])

            basic_url = extract_google_basic_results_url(body)
            if basic_url:
                response = await session.get(
                    basic_url,
                    headers=DEFAULT_HEADERS,
                    impersonate="chrome136",
                    timeout=15,
                    proxy=proxy,
                )
                body = response.text
                if len(raw_samples) < 2:
                    raw_samples.append(body[:1000])

            parsed = parse_google_basic_results(body, limit)
            if not parsed:
                continue

            successful_queries.append(search_query)
            for item in parsed:
                canonical = canonicalize_result_url(item.url)
                if canonical in seen:
                    continue
                seen.add(canonical)
                aggregated.append(item)
                if len(aggregated) >= limit:
                    break
            if len(aggregated) >= limit:
                break
    return {
        "success": bool(aggregated),
        "search_results": [asdict(item) for item in aggregated],
        "metadata": {
            "sidecar": "curl_cffi_cli",
            "proxy_used": bool(proxy),
            "queries_attempted": candidates,
            "queries_succeeded": successful_queries,
        },
        "raw_html": "\n\n---\n\n".join(raw_samples),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--query", required=True)
    parser.add_argument("--query-type", required=True, choices=["phone", "bank", "url"])
    parser.add_argument("--proxy")
    parser.add_argument("--limit", type=int, default=10)
    args = parser.parse_args()

    try:
        payload = asyncio.run(
            run_google_cli(args.query, args.query_type, args.proxy, args.limit)
        )
        print(json.dumps(payload, ensure_ascii=False))
        return 0
    except Exception as error:
        print(
            json.dumps(
                {
                    "success": False,
                    "error": str(error),
                    "search_results": [],
                    "metadata": {"sidecar": "curl_cffi_cli"},
                    "raw_html": "",
                },
                ensure_ascii=False,
            )
        )
        return 1


if __name__ == "__main__":
    sys.exit(main())
