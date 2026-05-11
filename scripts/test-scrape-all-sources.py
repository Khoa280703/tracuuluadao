#!/usr/bin/env python3
"""
Prototype: Parallel scraping test for all data sources.
Tests speed and data quality with optional proxy support.

Usage:
  python3 scripts/test-scrape-all-sources.py <phone> [--proxy <file>]
  python3 scripts/test-scrape-all-sources.py 0926408013
  python3 scripts/test-scrape-all-sources.py 0926408013 --proxy proxies/proxy_ola.md
"""

import asyncio
import time
import json
import sys
import re
import random
import argparse
from dataclasses import dataclass, field
from typing import Optional
from curl_cffi.requests import AsyncSession
from bs4 import BeautifulSoup


@dataclass
class ScrapeResult:
    source: str
    success: bool
    duration_ms: float
    data_summary: str
    raw_length: int = 0
    error: Optional[str] = None
    details: dict = field(default_factory=dict)


CHROME_IMPERSONATE = "chrome124"
DEFAULT_HEADERS = {
    "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    "Accept-Language": "vi-VN,vi;q=0.9,en;q=0.8",
}

STATIC_SLUGS = {
    "thong-tin", "lien-he", "marketing", "gioi-thieu", "bio",
    "to-cao-lua-dao", "chinh-sach-bao-mat", "dieu-khoan-su-dung",
    "huong-dan", "lien-ket", "tin-tuc", "hoi-dap",
}


def load_proxies(proxy_file: str) -> list[str]:
    """Load proxies from file. Supports formats:
    - host:port:user:pass (HTTP)
    - socks5h://user:pass@host:port (SOCKS5)
    """
    proxies = []
    with open(proxy_file) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            if line.startswith("socks5"):
                proxies.append(line)
            elif line.count(":") == 3:
                host, port, user, pwd = line.split(":")
                proxies.append(f"http://{user}:{pwd}@{host}:{port}")
            else:
                proxies.append(line)
    return proxies


def get_random_proxy(proxies: list[str]) -> Optional[str]:
    if not proxies:
        return None
    return random.choice(proxies)


async def scrape_checkscam(session: AsyncSession, query: str) -> ScrapeResult:
    """checkscam.vn — GET /?qh_ss=<query> → HTML parse report listings."""
    start = time.perf_counter()
    try:
        url = f"https://checkscam.vn/?qh_ss={query}"
        resp = await session.get(url, headers=DEFAULT_HEADERS, impersonate=CHROME_IMPERSONATE, timeout=15)
        elapsed = (time.perf_counter() - start) * 1000

        html = resp.text
        soup = BeautifulSoup(html, "html.parser")

        count_match = re.search(r"Có\s+(\d+)\s+cảnh báo", html)
        report_count = int(count_match.group(1)) if count_match else 0

        report_links = []
        seen_slugs = set()
        for a in soup.select("a[href]"):
            href = a.get("href", "")
            if "checkscam.vn/" not in href:
                continue
            slug = href.split("checkscam.vn/")[-1].rstrip("/")
            if (
                slug
                and "?" not in slug
                and "/" not in slug
                and "wp-" not in slug
                and slug not in STATIC_SLUGS
                and slug not in seen_slugs
            ):
                seen_slugs.add(slug)
                link_text = a.get_text(strip=True)
                if link_text and len(link_text) > 2:
                    report_links.append({"slug": slug, "text": link_text[:80], "url": href})

        return ScrapeResult(
            source="checkscam.vn",
            success=True,
            duration_ms=elapsed,
            data_summary=f"{report_count} cảnh báo, {len(report_links)} report links",
            raw_length=len(html),
            details={"report_count": report_count, "reports": report_links[:10]},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="checkscam.vn", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_chongluadao_phone(session: AsyncSession, query: str) -> ScrapeResult:
    """chongluadao.vn — GET /checkphone?q=<query> → JSON phone lookup."""
    start = time.perf_counter()
    try:
        url = f"https://feeds.chongluadao.vn/checkphone?q={query}"
        resp = await session.get(url, headers=DEFAULT_HEADERS, impersonate=CHROME_IMPERSONATE, timeout=15)
        elapsed = (time.perf_counter() - start) * 1000
        data = resp.json()

        has_data = False
        if isinstance(data, list):
            has_data = any(item.get("data") is not None for item in data if isinstance(item, dict))

        return ScrapeResult(
            source="chongluadao.vn/phone",
            success=True,
            duration_ms=elapsed,
            data_summary=f"Phone check: {len(data)} sources, {'has data' if has_data else 'no data'}",
            raw_length=len(resp.text),
            details={"data": data if isinstance(data, (dict, list)) else str(data)[:500]},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="chongluadao.vn/phone", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_chongluadao_check_exists(session: AsyncSession, query: str) -> ScrapeResult:
    """chongluadao.vn — GET /reports/check-exists?q=<query> → check if reported."""
    start = time.perf_counter()
    try:
        url = f"https://feeds.chongluadao.vn/reports/check-exists?q={query}"
        resp = await session.get(url, headers=DEFAULT_HEADERS, impersonate=CHROME_IMPERSONATE, timeout=15)
        elapsed = (time.perf_counter() - start) * 1000
        data = resp.json()
        return ScrapeResult(
            source="chongluadao.vn/reports",
            success=True,
            duration_ms=elapsed,
            data_summary=f"Report exists check: {json.dumps(data, ensure_ascii=False)[:100]}",
            raw_length=len(resp.text),
            details={"data": data},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="chongluadao.vn/reports", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_trangtrang(session: AsyncSession, query: str) -> ScrapeResult:
    """trangtrang.com — GET /<phone> → HTML parse phone info."""
    start = time.perf_counter()
    try:
        url = f"https://trangtrang.com/{query}"
        resp = await session.get(url, headers=DEFAULT_HEADERS, impersonate=CHROME_IMPERSONATE, timeout=15)
        elapsed = (time.perf_counter() - start) * 1000

        soup = BeautifulSoup(resp.text, "html.parser")

        main_content = soup.select_one("main, #__next, article")
        content_text = main_content.get_text(" ", strip=True)[:800] if main_content else ""

        carrier_match = re.search(r"(Viettel|Mobifone|Vinaphone|Vietnamobile|Gmobile|Reddi)", content_text, re.I)
        carrier = carrier_match.group(0) if carrier_match else "Unknown"

        warnings = []
        for el in soup.select("[class*='warn'], [class*='alert'], [class*='danger'], [class*='canh-bao']"):
            text = el.get_text(strip=True)[:100]
            if text:
                warnings.append(text)

        comments = []
        for el in soup.select("[class*='comment'], [class*='review'], [class*='feedback']"):
            text = el.get_text(strip=True)[:150]
            if text and len(text) > 10:
                comments.append(text)

        return ScrapeResult(
            source="trangtrang.com",
            success=True,
            duration_ms=elapsed,
            data_summary=f"Carrier: {carrier}, {len(warnings)} warnings, {len(comments)} comments",
            raw_length=len(resp.text),
            details={"carrier": carrier, "warnings": warnings[:5], "comments": comments[:5], "preview": content_text[:300]},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="trangtrang.com", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_tinnhiemmang(session: AsyncSession, query: str) -> ScrapeResult:
    """tinnhiemmang.vn — POST /searchOrg with XSRF token."""
    start = time.perf_counter()
    try:
        main_resp = await session.get("https://tinnhiemmang.vn", headers=DEFAULT_HEADERS, impersonate=CHROME_IMPERSONATE, timeout=15)
        xsrf_token = None
        for cookie in main_resp.cookies.jar:
            if cookie.name == "XSRF-TOKEN":
                xsrf_token = cookie.value
                break

        if not xsrf_token:
            elapsed = (time.perf_counter() - start) * 1000
            return ScrapeResult(source="tinnhiemmang.vn", success=False, duration_ms=elapsed, data_summary="", error="No XSRF token")

        search_headers = {
            **DEFAULT_HEADERS,
            "X-XSRF-TOKEN": xsrf_token,
            "Content-Type": "application/x-www-form-urlencoded",
            "Referer": "https://tinnhiemmang.vn/",
        }
        resp = await session.post(
            "https://tinnhiemmang.vn/searchOrg",
            data={"search": query},
            headers=search_headers,
            impersonate=CHROME_IMPERSONATE,
            timeout=15,
        )
        elapsed = (time.perf_counter() - start) * 1000

        soup = BeautifulSoup(resp.text, "html.parser")
        results = []
        for item in soup.select("a, tr, .result-item"):
            text = item.get_text(strip=True)[:100]
            href = item.get("href", "")
            if text and len(text) > 5:
                results.append({"text": text, "url": href} if href else {"text": text})

        return ScrapeResult(
            source="tinnhiemmang.vn",
            success=True,
            duration_ms=elapsed,
            data_summary=f"{len(results)} results",
            raw_length=len(resp.text),
            details={"results": results[:10]},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="tinnhiemmang.vn", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_google(session: AsyncSession, query: str, proxy: Optional[str] = None) -> ScrapeResult:
    """Google Search — with optional proxy support."""
    start = time.perf_counter()
    try:
        search_query = f"{query} lừa đảo"
        url = "https://www.google.com/search"
        params = {"q": search_query, "hl": "vi", "gl": "vn", "num": "10"}

        kwargs = dict(
            params=params,
            headers=DEFAULT_HEADERS,
            impersonate=CHROME_IMPERSONATE,
            timeout=15,
        )
        if proxy:
            kwargs["proxy"] = proxy

        resp = await session.get(url, **kwargs)
        elapsed = (time.perf_counter() - start) * 1000

        soup = BeautifulSoup(resp.text, "html.parser")

        results = []
        for g in soup.select("div.g"):
            title_el = g.select_one("h3")
            link_el = g.select_one("a[href]")
            snippet_el = g.select_one("span.VwiC3b, div.VwiC3b, div[data-sncf]")
            if title_el:
                results.append({
                    "title": title_el.get_text(strip=True),
                    "url": link_el.get("href", "") if link_el else "",
                    "snippet": snippet_el.get_text(strip=True)[:150] if snippet_el else "",
                })

        has_captcha = "captcha" in resp.text.lower() or "unusual traffic" in resp.text.lower()

        return ScrapeResult(
            source=f"google.com{' [proxy]' if proxy else ''}",
            success=not has_captcha and len(results) > 0,
            duration_ms=elapsed,
            data_summary=f"{len(results)} results" + (" [CAPTCHA]" if has_captcha else ""),
            raw_length=len(resp.text),
            details={"results": results[:5], "captcha": has_captcha, "proxy_used": bool(proxy)},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="google.com", success=False, duration_ms=elapsed, data_summary="", error=str(e))


async def scrape_duckduckgo(session: AsyncSession, query: str) -> ScrapeResult:
    """DuckDuckGo HTML — no proxy needed."""
    start = time.perf_counter()
    try:
        search_query = f"{query} lừa đảo"
        url = "https://html.duckduckgo.com/html/"
        data = {"q": search_query}
        ddg_headers = {
            **DEFAULT_HEADERS,
            "Content-Type": "application/x-www-form-urlencoded",
        }
        resp = await session.post(url, data=data, headers=ddg_headers, impersonate=CHROME_IMPERSONATE, timeout=15)
        elapsed = (time.perf_counter() - start) * 1000

        soup = BeautifulSoup(resp.text, "html.parser")

        results = []
        for r in soup.select(".result"):
            title_el = r.select_one(".result__title a, .result__a")
            snippet_el = r.select_one(".result__snippet")
            url_el = r.select_one(".result__url")
            if title_el:
                results.append({
                    "title": title_el.get_text(strip=True),
                    "url": url_el.get_text(strip=True) if url_el else "",
                    "snippet": snippet_el.get_text(strip=True)[:150] if snippet_el else "",
                })

        return ScrapeResult(
            source="duckduckgo.com",
            success=len(results) > 0,
            duration_ms=elapsed,
            data_summary=f"{len(results)} results",
            raw_length=len(resp.text),
            details={"results": results[:5]},
        )
    except Exception as e:
        elapsed = (time.perf_counter() - start) * 1000
        return ScrapeResult(source="duckduckgo.com", success=False, duration_ms=elapsed, data_summary="", error=str(e))


def print_result(result: ScrapeResult):
    status = "✅" if result.success else "❌"
    print(f"\n{'='*60}")
    print(f"{status} {result.source}")
    print(f"   Time: {result.duration_ms:.0f}ms | Size: {result.raw_length:,} bytes")
    print(f"   Summary: {result.data_summary}")
    if result.error:
        print(f"   Error: {result.error}")
    if result.details:
        for key, val in result.details.items():
            if isinstance(val, list) and val:
                print(f"   {key}:")
                for i, item in enumerate(val[:3]):
                    if isinstance(item, dict):
                        print(f"     [{i}] {json.dumps(item, ensure_ascii=False)[:120]}")
                    else:
                        print(f"     [{i}] {str(item)[:120]}")
                if len(val) > 3:
                    print(f"     ... +{len(val)-3} more")
            elif isinstance(val, str) and val:
                print(f"   {key}: {val[:150]}")
            elif isinstance(val, bool):
                print(f"   {key}: {val}")
            elif isinstance(val, (int, float)):
                print(f"   {key}: {val}")
            elif isinstance(val, dict) and val:
                print(f"   {key}: {json.dumps(val, ensure_ascii=False)[:150]}")


async def run_phone_test(phone: str, proxies: list[str]):
    print(f"\n{'#'*60}")
    print(f"# PHONE LOOKUP TEST: {phone}")
    if proxies:
        print(f"# Proxy pool: {len(proxies)} proxies loaded")
    print(f"{'#'*60}")

    total_start = time.perf_counter()
    proxy = get_random_proxy(proxies)

    async with AsyncSession() as session:
        tasks = [
            scrape_checkscam(session, phone),
            scrape_chongluadao_phone(session, phone),
            scrape_chongluadao_check_exists(session, phone),
            scrape_trangtrang(session, phone),
            scrape_tinnhiemmang(session, phone),
            scrape_google(session, phone, proxy=proxy),
            scrape_duckduckgo(session, phone),
        ]
        results = await asyncio.gather(*tasks, return_exceptions=True)

    total_elapsed = (time.perf_counter() - total_start) * 1000

    print("\n" + "=" * 60)
    print("RESULTS")
    print("=" * 60)

    success_count = 0
    for r in results:
        if isinstance(r, Exception):
            print(f"\n❌ Exception: {r}")
        else:
            print_result(r)
            if r.success:
                success_count += 1

    print(f"\n{'='*60}")
    print("SUMMARY")
    print(f"  Total time (parallel): {total_elapsed:.0f}ms")
    print(f"  Sources: {success_count}/{len(results)} successful")
    individual_times = [r.duration_ms for r in results if isinstance(r, ScrapeResult)]
    if individual_times:
        print(f"  Fastest: {min(individual_times):.0f}ms | Slowest: {max(individual_times):.0f}ms")
        print(f"  Sum (sequential): {sum(individual_times):.0f}ms")
        print(f"  Speedup: {sum(individual_times) / total_elapsed:.1f}x")
    print(f"{'='*60}")

    return results


async def main():
    parser = argparse.ArgumentParser(description="Parallel scrape test")
    parser.add_argument("query", nargs="?", default="0926408013", help="Phone/STK/URL to search")
    parser.add_argument("--proxy", "-p", help="Proxy file path")
    args = parser.parse_args()

    proxies = []
    if args.proxy:
        proxies = load_proxies(args.proxy)
        print(f"Loaded {len(proxies)} proxies from {args.proxy}")

    await run_phone_test(args.query, proxies)


if __name__ == "__main__":
    asyncio.run(main())
