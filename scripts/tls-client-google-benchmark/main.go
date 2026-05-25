package main

import (
	"encoding/json"
	"flag"
	"fmt"
	stdhtml "html"
	"io"
	"log"
	"net/url"
	"os"
	"strings"

	http "github.com/bogdanfinn/fhttp"
	tls_client "github.com/bogdanfinn/tls-client"
	"github.com/bogdanfinn/tls-client/profiles"
	"golang.org/x/net/html"
)

const chrome144UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/144.0.0.0 Safari/537.36"

type searchResult struct {
	Title string `json:"title"`
	URL   string `json:"url"`
}

type fetchResult struct {
	Label       string         `json:"label"`
	URL         string         `json:"url"`
	StatusCode  int            `json:"status_code"`
	BodyLength  int            `json:"body_length"`
	Blocked     bool           `json:"blocked"`
	CaptchaForm bool           `json:"captcha_form"`
	EnableJS    bool           `json:"enablejs"`
	ResultCount int            `json:"result_count"`
	Results     []searchResult `json:"results"`
	Preview     string         `json:"preview"`
}

func main() {
	query := flag.String("query", "0562015037", "search query")
	proxyURL := flag.String("proxy", "", "optional proxy URL")
	limit := flag.Int("limit", 10, "max parsed results per request")
	flag.Parse()

	client, err := newClient(*proxyURL)
	if err != nil {
		log.Fatal(err)
	}

	runs := []struct {
		label  string
		params url.Values
	}{
		{"plain", url.Values{"q": {*query}, "hl": {"vi"}, "gl": {"vn"}, "num": {"10"}}},
		{"tch1", url.Values{"q": {*query}, "hl": {"vi"}, "gl": {"vn"}, "tch": {"1"}, "num": {"10"}}},
		{"gbv1", url.Values{"q": {*query}, "hl": {"vi"}, "gl": {"vn"}, "gbv": {"1"}, "num": {"10"}}},
	}

	var output []fetchResult
	for _, run := range runs {
		target := "https://www.google.com/search?" + run.params.Encode()
		result, err := fetch(client, run.label, target, *limit)
		if err != nil {
			log.Fatalf("%s failed: %v", run.label, err)
		}
		output = append(output, result)
	}

	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(output); err != nil {
		log.Fatal(err)
	}
}

func newClient(proxyURL string) (tls_client.HttpClient, error) {
	options := []tls_client.HttpClientOption{
		tls_client.WithTimeoutSeconds(20),
		tls_client.WithClientProfile(profiles.Chrome_144),
		tls_client.WithCookieJar(tls_client.NewCookieJar()),
	}
	if proxyURL != "" {
		options = append(options, tls_client.WithProxyUrl(proxyURL))
	}
	return tls_client.NewHttpClient(tls_client.NewNoopLogger(), options...)
}

func fetch(client tls_client.HttpClient, label, target string, limit int) (fetchResult, error) {
	req, err := http.NewRequest(http.MethodGet, target, nil)
	if err != nil {
		return fetchResult{}, err
	}
	req.Header = http.Header{
		"accept":                    {"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"},
		"accept-language":           {"vi-VN,vi;q=0.9,en;q=0.8"},
		"priority":                  {"u=0, i"},
		"sec-ch-ua":                 {`"Chromium";v="144", "Google Chrome";v="144", "Not)A;Brand";v="24"`},
		"sec-ch-ua-mobile":          {"?0"},
		"sec-ch-ua-platform":        {`"Linux"`},
		"sec-fetch-dest":            {"document"},
		"sec-fetch-mode":            {"navigate"},
		"sec-fetch-site":            {"none"},
		"sec-fetch-user":            {"?1"},
		"upgrade-insecure-requests": {"1"},
		"user-agent":                {chrome144UA},
		http.HeaderOrderKey: {
			"sec-ch-ua",
			"sec-ch-ua-mobile",
			"sec-ch-ua-platform",
			"upgrade-insecure-requests",
			"user-agent",
			"accept",
			"sec-fetch-site",
			"sec-fetch-mode",
			"sec-fetch-user",
			"sec-fetch-dest",
			"accept-language",
			"priority",
		},
	}

	resp, err := client.Do(req)
	if err != nil {
		return fetchResult{}, err
	}
	defer resp.Body.Close()

	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		return fetchResult{}, err
	}
	body := string(bodyBytes)
	results := extractResults(body, limit)

	return fetchResult{
		Label:       label,
		URL:         target,
		StatusCode:  resp.StatusCode,
		BodyLength:  len(body),
		Blocked:     isBlocked(body),
		CaptchaForm: strings.Contains(strings.ToLower(body), "captcha-form"),
		EnableJS:    strings.Contains(body, "/httpservice/retry/enablejs"),
		ResultCount: len(results),
		Results:     results,
		Preview:     preview(body, 300),
	}, nil
}

func extractResults(body string, limit int) []searchResult {
	root, err := html.Parse(strings.NewReader(body))
	if err != nil {
		return nil
	}
	var results []searchResult
	var visit func(*html.Node)
	visit = func(node *html.Node) {
		if node == nil || len(results) >= limit {
			return
		}
		if node.Type == html.ElementNode && node.Data == "h3" {
			title := strings.TrimSpace(textContent(node))
			href := ancestorHref(node)
			if title != "" && href != "" {
				results = append(results, searchResult{
					Title: stdhtml.UnescapeString(title),
					URL:   normalizeGoogleURL(href),
				})
			}
		}
		for child := node.FirstChild; child != nil; child = child.NextSibling {
			visit(child)
		}
	}
	visit(root)
	return dedupeResults(results, limit)
}

func ancestorHref(node *html.Node) string {
	for current := node.Parent; current != nil; current = current.Parent {
		if current.Type != html.ElementNode || current.Data != "a" {
			continue
		}
		for _, attr := range current.Attr {
			if attr.Key == "href" {
				return attr.Val
			}
		}
	}
	return ""
}

func textContent(node *html.Node) string {
	var builder strings.Builder
	var visit func(*html.Node)
	visit = func(current *html.Node) {
		if current.Type == html.TextNode {
			builder.WriteString(current.Data)
			builder.WriteByte(' ')
		}
		for child := current.FirstChild; child != nil; child = child.NextSibling {
			visit(child)
		}
	}
	visit(node)
	return builder.String()
}

func normalizeGoogleURL(href string) string {
	if strings.HasPrefix(href, "/url?") {
		if parsed, err := url.Parse("https://www.google.com" + href); err == nil {
			if target := parsed.Query().Get("q"); target != "" {
				return target
			}
		}
	}
	return href
}

func dedupeResults(results []searchResult, limit int) []searchResult {
	seen := map[string]bool{}
	var deduped []searchResult
	for _, item := range results {
		if seen[item.URL] {
			continue
		}
		seen[item.URL] = true
		deduped = append(deduped, item)
		if len(deduped) >= limit {
			break
		}
	}
	return deduped
}

func isBlocked(body string) bool {
	lower := strings.ToLower(body)
	return strings.Contains(lower, "detected unusual traffic") ||
		strings.Contains(lower, "g-recaptcha") ||
		strings.Contains(lower, "/sorry/index") ||
		strings.Contains(lower, "id=\"captcha\"") ||
		strings.Contains(lower, "name=\"captcha\"") ||
		strings.Contains(lower, "captcha-form") ||
		strings.Contains(lower, "/httpservice/retry/enablejs")
}

func preview(body string, max int) string {
	body = strings.ReplaceAll(body, "\n", " ")
	body = strings.ReplaceAll(body, "\r", " ")
	body = strings.Join(strings.Fields(body), " ")
	if len(body) <= max {
		return body
	}
	return fmt.Sprintf("%s...", body[:max])
}
