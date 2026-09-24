#!/usr/bin/env python3
"""Build a Commons SVG blank-map catalog and review candidate files."""
import argparse, csv, email.utils, html, http.client, json, re, time, urllib.error, urllib.parse, urllib.request
from collections import defaultdict, deque
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path

API = "https://commons.wikimedia.org/w/api.php"
ROOT = "Category:SVG blank maps"
UA = "MapGeneratorSvgBlankMapsCatalog/1.0 (https://github.com/schiste/map-generator)"
_LAST_QUERY = 0.0
_CONNECTION = None

class Text(HTMLParser):
    def __init__(self):
        super().__init__(); self.parts = []
    def handle_data(self, data):
        self.parts.append(data)

def clean(value):
    parser = Text(); parser.feed(html.unescape(str(value or "")))
    text = re.sub(r"\s+", " ", " ".join(parser.parts)).strip()
    return re.sub(r"\s+([,.;:!?])", r"\1", text)

def query(params):
    global _LAST_QUERY, _CONNECTION
    pause = 0.5 - (time.monotonic() - _LAST_QUERY)
    if pause > 0: time.sleep(pause)
    _LAST_QUERY = time.monotonic()
    values = {"action":"query", "format":"json", "formatversion":"2", "maxlag":"5", **params}
    path = "/w/api.php?" + urllib.parse.urlencode(values)
    last = None
    for attempt in range(6):
        try:
            if _CONNECTION is None:
                _CONNECTION = http.client.HTTPSConnection("commons.wikimedia.org", timeout=45)
            _CONNECTION.request("GET", path, headers={"User-Agent":UA, "Accept":"application/json"})
            response = _CONNECTION.getresponse()
            body = response.read()
            if response.status in {429,503}:
                last = RuntimeError(f"HTTP {response.status}: {body[:300]!r}")
                try: retry_after = int(response.getheader("Retry-After", "60") or 60)
                except ValueError: retry_after = 60
                _CONNECTION.close()
                _CONNECTION = None
                if attempt < 5:
                    time.sleep(max(retry_after, 30))
                    continue
                break
            if response.status == 403:
                last = RuntimeError(f"HTTP 403: {body[:300]!r}")
                try: retry_after = int(response.getheader("Retry-After", "300") or 300)
                except ValueError: retry_after = 300
                _CONNECTION.close()
                _CONNECTION = None
                if attempt < 2:
                    time.sleep(max(retry_after, 300))
                    continue
                break
            if response.status >= 400:
                last = RuntimeError(f"HTTP {response.status}: {body[:300]!r}")
                _CONNECTION.close()
                _CONNECTION = None
                if attempt < 5:
                    time.sleep(min(5 * (2 ** attempt), 60))
                    continue
                break
            data = json.loads(body)
            api_error = data.get("error", {})
            if api_error.get("code") in {"maxlag","ratelimited"}:
                last = RuntimeError(f"Commons API error: {api_error}")
                try: delay = max(5,float(api_error.get("lag",0)))
                except (TypeError,ValueError): delay = 5
                if response.will_close:
                    _CONNECTION.close()
                    _CONNECTION = None
                if attempt < 5:
                    time.sleep(delay)
                    continue
                break
            if response.will_close:
                _CONNECTION.close()
                _CONNECTION = None
            return data
        except (OSError, http.client.HTTPException, json.JSONDecodeError) as exc:
            last = exc
            if _CONNECTION is not None:
                _CONNECTION.close()
                _CONNECTION = None
            if attempt < 5: time.sleep(min(5 * (2 ** attempt), 60))
    raise RuntimeError(f"Commons API request failed: {last}")

def pages(params):
    continuation = {}
    while True:
        response = query({**params, **continuation})
        if response.get("error"):
            raise RuntimeError(f"Commons API error: {response['error']}")
        yield response
        continuation = response.get("continue", {})
        if not continuation: break

def direct_members(category):
    children, files = set(), set()
    for response in pages({"list":"categorymembers", "cmtitle":category, "cmtype":"subcat|file", "cmlimit":"500"}):
        for member in response.get("query",{}).get("categorymembers",[]):
            if member.get("ns") == 14: children.add(member["title"])
            elif member.get("ns") == 6: files.add(member["title"])
    return children, files

def direct_subtree(root):
    """Recursively enumerate a failed deepcat branch using categorymembers."""
    categories = {root}
    parents = defaultdict(set)
    members = defaultdict(set)
    queue = deque([root])
    seen = set()
    while queue:
        category = queue.popleft()
        if category in seen:
            continue
        seen.add(category)
        children, files = direct_members(category)
        for title in files:
            members[title].add(category)
        for child in children:
            categories.add(child)
            parents[child].add(category)
            queue.append(child)
    return categories, parents, members

def walk(root, cache_path=None, reuse_cache=False):
    if reuse_cache and cache_path and cache_path.exists():
        cached = json.loads(cache_path.read_text(encoding="utf-8"))
        categories = set(cached["categories"])
        parents = defaultdict(set, {k:set(v) for k,v in cached["parents"].items()})
        files = defaultdict(set, {k:set(v) for k,v in cached["members"].items()})
        print(f"Reused category cache: {len(categories):,} categories, {len(files):,} files",flush=True)
        return categories, parents, files, cached["branch_count"], cached["warnings"]
    root_children, root_files = direct_members(root)
    country_root = next((c for c in root_children if c.casefold() == "category:svg blank maps by country"), None)
    country_children = set()
    files = defaultdict(set)
    parents = defaultdict(set)
    for title in root_files: files[title].add(root)
    for child in root_children: parents[child].add(root)
    branches = set(root_children)
    if country_root:
        country_children, country_files = direct_members(country_root)
        for title in country_files: files[title].add(country_root)
        for child in country_children: parents[child].add(country_root)
        branches.discard(country_root)
        branches.update(country_children)

    # Split the two map-template hubs so every deepcat search remains below
    # the documented 256-category limit, even if individual countries have
    # nested administrative map categories.
    for hub in list(branches):
        if hub.casefold() in {"category:svg locator maps by country", "category:svg location maps by country"}:
            hub_children, hub_files = direct_members(hub)
            for title in hub_files: files[title].add(hub)
            for child in hub_children: parents[child].add(hub)
            branches.discard(hub)
            branches.update(hub_children)

    branches = sorted(branches, key=str.casefold)
    warnings = []
    fallback_categories_all = set()
    for index, branch in enumerate(branches, 1):
        category_name = branch.removeprefix("Category:")
        search = f'deepcat:"{category_name}"'
        for response in pages({"list":"search","srsearch":search,"srnamespace":"6","srlimit":"500"}):
            if response.get("error"):
                raise RuntimeError(f"Deep-category search failed for {branch}: {response['error']}")
            if response.get("warnings"):
                warning_text = json.dumps(response["warnings"], ensure_ascii=False)
                if "Deep category search SPARQL query failed" in warning_text:
                    print(f"Falling back to recursive categorymembers for {branch}",flush=True)
                    fallback_categories, fallback_parents, fallback_files = direct_subtree(branch)
                    fallback_categories_all.update(fallback_categories)
                    for category, ancestors in fallback_parents.items():
                        parents[category].update(ancestors)
                    for title, matched_categories in fallback_files.items():
                        files[title].update(matched_categories)
                        files[title].add(branch)
                    warnings.append(f"{branch}: deepcat SPARQL failed; recursive categorymembers fallback covered {len(fallback_categories):,} categories")
                    break
                warnings.append(f"{branch}: {warning_text}")
            for result in response.get("query",{}).get("search",[]):
                files[result["title"]].add(branch)
        if index % 10 == 0:
            print(f"Completed {index:,}/{len(branches):,} recursive search branches; found {len(files):,} unique files", flush=True)
    categories = {root} | root_children | country_children | fallback_categories_all
    for branch in branches:
        categories.add(branch)
    for warning in warnings:
        print(f"Deep-category warning: {warning}", flush=True)
    print(f"Recursive branch warnings: {len(warnings)}", flush=True)
    if cache_path:
        cache_path.parent.mkdir(parents=True,exist_ok=True)
        cache_path.write_text(json.dumps({
            "categories":sorted(categories,key=str.casefold),
            "parents":{k:sorted(v,key=str.casefold) for k,v in parents.items()},
            "members":{k:sorted(v,key=str.casefold) for k,v in files.items()},
            "branch_count":len(branches),"warnings":warnings
        },ensure_ascii=False),encoding="utf-8")
        print(f"Saved category cache: {cache_path}",flush=True)
    return categories, parents, files, len(branches), warnings

def metadata_for(titles, cache_dir):
    out = {}
    cache_dir.mkdir(parents=True,exist_ok=True)
    batches = (len(titles)+49)//50
    for batch_index, offset in enumerate(range(0,len(titles),50),1):
        batch = titles[offset:offset+50]
        batch_path = cache_dir/f"metadata-{batch_index:05d}.json"
        if batch_path.exists():
            saved = json.loads(batch_path.read_text(encoding="utf-8"))
            for title,item in saved.items():
                item["categories"] = set(item.get("categories",[]))
                item["globalusage_count"] = len(item.get("globalusage",[]))
                item.pop("globalusage",None)
                out[title] = item
            if batch_index % 20 == 0 or batch_index == batches:
                print(f"Loaded cached metadata batches: {batch_index:,}/{batches:,}",flush=True)
            continue
        print(f"Fetching metadata batch {batch_index:,}/{batches:,} ({len(batch)} files)",flush=True)
        batch_out = {}
        page_count = 0
        for response in pages({"prop":"imageinfo|categories|globalusage","titles":"|".join(batch),
                               "iiprop":"url|size|sha1|mime|extmetadata","iiurlwidth":"320",
                               "iiextmetadatalanguage":"en","cllimit":"max","gulimit":"max",
                               "guprop":"pageid"}):
            page_count += 1
            for page in response.get("query",{}).get("pages",[]):
                item = batch_out.setdefault(page["title"],{"categories":set(),"imageinfo":None,"missing":False,"globalusage":set()})
                item["missing"] = bool(page.get("missing"))
                item["categories"].update(c["title"] for c in page.get("categories",[]))
                if page.get("imageinfo"): item["imageinfo"] = page["imageinfo"][0]
                for usage in page.get("globalusage",[]):
                    item["globalusage"].add((usage.get("wiki",""),str(usage.get("pageid",usage.get("title","")))))
            if page_count % 10 == 0:
                print(f"  batch {batch_index:,}: processed {page_count:,} continuation pages",flush=True)
        serializable = {title:{**item,"categories":sorted(item["categories"],key=str.casefold),
                                "globalusage":[list(x) for x in sorted(item["globalusage"])]}
                        for title,item in batch_out.items()}
        batch_path.write_text(json.dumps(serializable,ensure_ascii=False),encoding="utf-8")
        page_uses = sum(len(x["globalusage"]) for x in batch_out.values())
        for item in batch_out.values():
            item["globalusage_count"] = len(item["globalusage"])
            item["globalusage"] = set()
        out.update(batch_out)
        print(f"Saved metadata batch {batch_index:,}/{batches:,} with {page_uses:,} page uses",flush=True)
    return out

def usage_for(titles):
    out = defaultdict(set)
    for offset in range(0,len(titles),50):
        batch = titles[offset:offset+50]
        for response in pages({"prop":"globalusage","titles":"|".join(batch),"gulimit":"max"}):
            for page in response.get("query",{}).get("pages",[]):
                for item in page.get("globalusage",[]):
                    out[page["title"]].add((item.get("wiki",""),item.get("title","")))
        if offset and offset % 1000 == 0: print(f"Global usage: {min(offset+50,len(titles)):,}/{len(titles):,}",flush=True)
    return out

def probe(url):
    if not url: return {"status":"no preview URL","http":"","type":""}
    for attempt in range(3):
        req = urllib.request.Request(url,method="HEAD",headers={"User-Agent":UA})
        try:
            with urllib.request.urlopen(req,timeout=25) as r:
                return {"status":"reachable" if r.status==200 else "HTTP error","http":r.status,"type":r.headers.get("Content-Type","")}
        except urllib.error.HTTPError as e:
            if e.code in {429,500,502,503,504} and attempt < 2:
                retry_after=e.headers.get("Retry-After","")
                try: delay=max(5,int(retry_after))
                except ValueError:
                    try: delay=max(5,email.utils.parsedate_to_datetime(retry_after).timestamp()-time.time())
                    except (TypeError,ValueError,OverflowError): delay=10
                time.sleep(delay)
                continue
            return {"status":"HTTP error","http":e.code,"type":e.headers.get("Content-Type","")}
        except Exception as e:
            if attempt < 2:
                time.sleep(2)
                continue
            return {"status":"probe failed: "+type(e).__name__,"http":"","type":""}

def depth(cat, parents, root, memo, seen=None):
    if cat == root: return 0
    if cat in memo: return memo[cat]
    seen = set() if seen is None else seen
    if cat in seen: return 1
    seen.add(cat)
    values = [depth(p,parents,root,memo,seen.copy()) for p in parents.get(cat,())]
    memo[cat] = 1 + max(values) if values else 1
    return memo[cat]

def category_area(cat):
    label = cat.removeprefix("Category:")
    if re.search(r"\bmaps?\s+by\s+(country|region|continent|type)\b",label,re.I): return ""
    m = re.search(r"\b(?:blank\s+|locator\s+|location\s+|topographic\s+|political\s+|physical\s+|historic(?:al)?\s+|SVG\s+)*maps?\s+(?:of|for|in|showing|covering|around)\s+(.+)",label,re.I)
    if not m: return ""
    area=m.group(1).strip()
    while re.search(r"\s*\([^)]*\)\s*$",area):
        area=re.sub(r"\s*\([^)]*\)\s*$","",area).strip()
    return area.strip(" .,: ")

AREAS = ["world","africa","antarctica","asia","europe","oceania","north america","south america",
         "central america","caribbean","middle east","arab world","afro-eurasia","eurasia",
         "united states","united kingdom","new zealand","south korea","north korea","czech republic",
         "state of palestine","honduras","ethiopia","philippines","kazakhstan","kyrgyzstan",
         "australia","austria","belgium","burkina faso","canada","china","colombia","ecuador",
         "france","germany","haiti","india","israel","italy","japan","mexico","montenegro",
         "pakistan","poland","portugal","romania","slovakia","spain","switzerland","ukraine","uruguay"]

def area_from_text(value):
    text = clean(value)
    for pattern in [r"\b(?:blank\s+)?maps?\s+(?:of|for|in|showing|covering)\s+(.+?)(?:[.;]|$)",
                    r"\bmap\s*[-_:]+\s*([A-Za-z].+?)(?:[.;]|$)"]:
        m = re.search(pattern,text,re.I)
        if m: return re.sub(r"\s*\([^)]*\)\s*$","",m.group(1)).strip(" .,:")
    for name in AREAS:
        if re.search(r"\b"+re.escape(name)+r"\b",text,re.I): return name.title()
    return ""

WIDE_AREAS={x.casefold() for x in AREAS}|{"soviet union","russian empire","ottoman empire","holy roman empire","austro-hungarian empire","yugoslavia","kingdom of hungary","kingdom of italy"}
AREA_QUALIFIERS=("map","grid","rivers","relief","zoom","marker","claimed","disputed","de-facto","semi-secession","natural","blue marble","orthographic","mini","scheme","plus ","pop areas","special","view","w3")

def broad_area(value,extra_areas=()):
    value=re.sub(r"\s*\([^)]*\)\s*$","",value or "").strip()
    value=re.sub(r"^\d{3,4}[–-]\d{3,4}\s+","",value)
    allowed=WIDE_AREAS|{x.casefold().removeprefix("the ") for x in extra_areas}
    normalized=value.casefold().removeprefix("the ")
    if normalized in allowed: return value
    match=re.search(r"\b(?:of|in)\s+(.+)$",value,re.I)
    if match:
        candidate=match.group(1).strip().casefold().removeprefix("the ")
        if candidate in allowed: return match.group(1).strip()
    return ""

def coverage_from_title(filename,base_area,extra_areas=()):
    stem=re.sub(r"\.(?:svgz?|png|gif|jpe?g)$","",filename,flags=re.I).replace("_"," ").strip()
    match=re.match(r"^(.+?)\s+in\s+(.+)$",stem,re.I)
    if not match:
        globe=re.match(r"^(.+?)\s+on\s+the\s+globe\b",stem,re.I)
        if not globe: return ""
        subject=globe.group(1).strip(" ,")
        parent=broad_area(base_area,extra_areas)
        if not parent and extra_areas:
            parent=sorted(extra_areas,key=str.casefold)[0]
        return subject if not parent or subject.casefold()==parent.casefold() else f"{subject}, {parent}"
    subject=match.group(1).strip()
    subject=re.sub(r"^(?:(?:blank|outline|political|administrative|physical|historical)\s+)?(?:(?:locator|location)\s+)?map\s+of\s+","",subject,flags=re.I).strip(" ,")
    if not subject or re.search(r"\b(?:map|maps|blank|locator|location)\b",subject,re.I): return ""
    region=match.group(2).strip()
    region=re.sub(r"\s+(?:-|–)\s*\d{4}$","",region)
    while True:
        qualifier=re.search(r"\s*\(([^()]*)\)\s*$",region)
        if not qualifier: break
        label=qualifier.group(1).strip().casefold()
        if any(token in label for token in AREA_QUALIFIERS) or re.fullmatch(r"(?:us\d+|w\d+|[a-z]{2,4})",label):
            region=region[:qualifier.start()].strip()
        else:
            break
    region=region.strip(" ,.;")
    if re.fullmatch(r"(?:ca\.?\s*)?\d{3,4}(?:\s*(?:bc|ad|bce|ce))?",region,re.I): region=""
    explicit_parent=""
    parenthetical=re.fullmatch(r"(.+?)\s+\(([^()]+)\)",region)
    if parenthetical and parenthetical.group(2).strip().casefold().removeprefix("the ") in WIDE_AREAS:
        region=parenthetical.group(1).strip()
        explicit_parent=parenthetical.group(2).strip()
    base=broad_area(base_area,extra_areas)
    if re.fullmatch(r"(?:its|their|this)\s+(?:region|location)",region,re.I): region=base
    if not region: region=base
    if not region: return ""
    if subject.casefold()==region.casefold(): return region
    pieces=[subject,region]
    parent=explicit_parent or base
    combined=" ".join(pieces).casefold()
    normalized_parent=parent.casefold().removeprefix("the ")
    if parent and normalized_parent not in combined and region.casefold() not in WIDE_AREAS:
        pieces.append(parent)
    return ", ".join(pieces)

def area_for(title,desc,found,allcats,parents,root):
    if title.casefold() == "blank map.svg":
        return "Generic locator template (no fixed geographic region)"
    memo = {}; choices = []
    for cat in set(found)|set(allcats):
        area = category_area(cat)
        area = re.split(r"\s+by\s+User:",area,flags=re.I)[0].strip()
        if area:
            group_area=bool(re.match(r"^(?:countries|capitals|places|locations)(?:\s+of\b|$)",area,re.I))
            choices.append((int(not group_area),len(area),depth(cat,parents,root,memo),area,cat))
    generic_region=bool(re.search(r"\bin\s+(?:its|their|this)\s+region\b",title,re.I))
    country_choices=[item for item in choices if re.search(r"\(country\)",item[4],re.I)]
    continents={"world","africa","antarctica","asia","europe","oceania","north america","south america","central america","caribbean","middle east","arab world","afro-eurasia","eurasia"}
    known_country_choices=[item for item in choices if item[3].casefold().removeprefix("the ") in WIDE_AREAS and item[3].casefold().removeprefix("the ") not in continents]
    parent_choices=country_choices or known_country_choices
    extra_areas={item[3] for item in parent_choices}
    if generic_region and country_choices:
        base=sorted(country_choices,key=lambda item:(item[2],len(item[3]),item[3].casefold()))[-1][3]
    else:
        base=sorted(choices,reverse=True)[0][3] if choices else area_from_text(title) or area_from_text(desc) or "Unclear from title/categories"
    return coverage_from_title(title,base,extra_areas) or base

def short_desc(filename,area,desc,categories=()):
    if filename.casefold() == "blank map.svg":
        return "Generic locator template; it does not identify a fixed geographic region or country boundaries."
    stem = re.sub(r"\.svgz?$","",filename,flags=re.I).replace("_"," ")
    stem = re.sub(r"-+"," ",stem).strip()
    is_locator=bool(re.search(r"location\s+map|locator\s+map",stem,re.I)) or any(re.search(r"location\s+maps?|locator\s+maps?",c,re.I) for c in categories)
    if desc:
        desc=clean(desc)
        generated_locator=bool(re.match(r"Locator map for .*; shows the named area within a wider region\.$",desc,re.I))
        if is_locator and (generated_locator or re.search(r"processed using|qgis atlas|generated using",desc,re.I) or re.search(r"\b(?:see filename|\bxy\b)",desc,re.I)):
            return f"Locator map for {area}; shows the named area within a wider region."
        if len(desc)<=420: return desc
        prefix=re.split(r"\b(?:processed using|generated using|based on)\b",desc,maxsplit=1,flags=re.I)[0].strip(" ,;.")
        if prefix and len(prefix)<=420: return prefix
        prefix=desc[:417].rsplit(" ",1)[0].strip() or desc[:417].strip()
        return prefix+"..."
    if area != "Unclear from title/categories":
        if is_locator:
            return f"Locator map for {area}; shows the named area within a wider region."
        if re.search(r"topograph|physical|relief|terrain|elevation",stem,re.I):
            return f"Physical or topographic blank map of {area}; see the source categories for detail."
        if re.search(r"historic|historical|century|\b(?:1[5-9]|20)\d{2}\b",stem,re.I):
            return f"Historical blank map of {area}; see the title and source categories for its period and scope."
        if re.search(r"political|administrative|\b(?:region|province|department|county|district|state)\b",stem,re.I):
            return f"Political or administrative blank map of {area}; see source categories for boundary detail."
        return f"Outline map of {area}, intended for adding labels or data; see categories for its scope."
    return f"Blank-map file titled “{stem}”; its coverage is unclear from Commons metadata."

def dup_key(filename):
    tokens = re.findall(r"[a-z0-9]+",re.sub(r"\.[^.]+$","",filename.lower()))
    return " ".join(sorted(t for t in tokens if t not in {"blank","map","maps","blankmap","svg","the","of","for","with","and","file","a"}))

def file_url(title):
    return "https://commons.wikimedia.org/wiki/"+urllib.parse.quote(title.replace(" ","_"),safe=":_()-.,'")

def usage_url(title):
    return "https://commons.wikimedia.org/wiki/Special:GlobalUsage/"+urllib.parse.quote(title.replace(" ","_"),safe=":_()-.,'")

def csvout(path,rows,fields):
    path.parent.mkdir(parents=True,exist_ok=True)
    with path.open("w",encoding="utf-8-sig",newline="") as f:
        w=csv.DictWriter(f,fieldnames=fields,extrasaction="ignore"); w.writeheader(); w.writerows(rows)

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--root-category",default=ROOT)
    parser.add_argument("--output-dir",default="reports/svg-blank-maps")
    parser.add_argument("--skip-thumbnail-probes",action="store_true")
    parser.add_argument("--reuse-category-cache",action="store_true")
    args=parser.parse_args()
    outdir=Path(args.output_dir)
    print(f"Walking Commons category tree from {args.root_category}...",flush=True)
    cats,parents,members,branch_count,branch_warnings=walk(args.root_category,outdir/"category-members.json",args.reuse_category_cache)
    titles=sorted(members,key=str.casefold)
    print(f"Found {len(cats):,} categories and {len(titles):,} unique files; reading metadata...",flush=True)
    metadata=metadata_for(titles,outdir/"cache")
    print("GlobalUsage counts were fetched in the same metadata batches.",flush=True)
    usage_counts={title:item.get("globalusage_count",0) for title,item in metadata.items()}
    probes={}
    if not args.skip_thumbnail_probes:
        urls={t:(metadata.get(t,{}).get("imageinfo") or {}).get("thumburl","") for t in titles}
        print(f"Checking {sum(bool(x) for x in urls.values()):,} thumbnail URLs...",flush=True)
        with ThreadPoolExecutor(max_workers=3) as pool:
            futures={pool.submit(probe,url):title for title,url in urls.items() if url}
            for index,f in enumerate(as_completed(futures),1):
                probes[futures[f]]=f.result()
                if index % 1000 == 0 or index == len(futures):
                    print(f"Checked thumbnail URLs: {index:,}/{len(futures):,}",flush=True)
    else: probes={t:{"status":"not probed","http":"","type":""} for t in titles}

    rows=[]; issues=[]; hashes=defaultdict(list); keys=defaultdict(list)
    for title in titles:
        item=metadata.get(title,{})
        info=item.get("imageinfo") or {}
        ext=info.get("extmetadata") or {}
        desc=clean((ext.get("ImageDescription") or {}).get("value",""))
        found=sorted(members.get(title,()),key=str.casefold)
        allcats=sorted(item.get("categories",()),key=str.casefold)
        filename=title.removeprefix("File:")
        area=area_for(filename,desc,found,allcats,parents,args.root_category)
        preview=probes.get(title,{"status":"no preview URL","http":"","type":""})
        thumb=info.get("thumburl","")
        invalid_svg_cats=[c for c in allcats if c.casefold().startswith("category:invalid svg")]
        broken_link_cats=[c for c in allcats if "category:files with broken file links" in c.casefold()]
        issue_parts=[]
        if invalid_svg_cats: issue_parts.append("Commons category marks SVG as invalid")
        if broken_link_cats: issue_parts.append("Commons category reports broken file links")
        if item.get("missing") or not info: issue_parts.append("Commons file page is missing image metadata")
        elif not thumb and info.get("mime")=="image/svg+xml": issue_parts.append("Commons did not return an SVG preview URL")
        if preview["status"]=="HTTP error" and str(preview["http"])=="429":
            preview["status"]="rate limited (HTTP 429; reachability unconfirmed)"
            issue_parts.append("Thumbnail request was rate-limited (HTTP 429); reachability unconfirmed")
        elif preview["status"] not in {"reachable","not probed","no preview URL"}: issue_parts.append("Preview request failed: "+preview["status"])
        elif preview["status"]=="reachable" and preview["type"] and not preview["type"].lower().startswith("image/"): issue_parts.append("Preview returned a non-image content type")
        issue="; ".join(issue_parts)
        rows.append({
            "file":filename,"geographic_coverage":area,"short_description":short_desc(filename,area,desc,allcats),
            "matched_descendant_branches":" | ".join(found),"commons_categories":" | ".join(allcats),
            "usage_count_wikimedia_pages":usage_counts.get(title,0),
            "usage_count_basis":"distinct (wiki, page ID) pairs returned by Commons GlobalUsage",
            "mime_type":info.get("mime",""),"width_px":info.get("width",""),"height_px":info.get("height",""),
            "file_size_bytes":info.get("size",""),"sha1":info.get("sha1",""),
            "thumbnail_probe":preview["status"],"thumbnail_http_status":preview["http"],
            "thumbnail_content_type":preview["type"],"possible_render_issue":issue,
            "duplicate_review_candidates":"","commons_file_page":file_url(title),"global_usage_page":usage_url(title)})
        if issue: issues.append({"file":filename,"geographic_coverage":area,"possible_issue":issue,"evidence_categories":" | ".join(invalid_svg_cats+broken_link_cats),"mime_type":info.get("mime",""),"thumbnail_http_status":preview["http"],"commons_file_page":file_url(title)})
        if info.get("sha1"): hashes[info["sha1"]].append((title,area))
        key=dup_key(filename)
        if key: keys[(area.casefold(),key)].append((title,info.get("width"),info.get("height"),info.get("sha1")))

    duplicate_rows=[]
    for sha,items in hashes.items():
        if len(items)>1:
            for i,(a,area) in enumerate(items):
                for b,_ in items[i+1:]:
                    duplicate_rows.append({"finding_type":"identical source-file SHA-1","file_a":a.removeprefix("File:"),"file_b":b.removeprefix("File:"),"geographic_coverage":area,"evidence":f"Commons reports identical SHA-1 {sha}"})
    for (area,key),items in keys.items():
        for i,a in enumerate(items):
            for b in items[i+1:]:
                if a[3] and a[3]==b[3]: continue
                ratio=0.0
                if a[1] and a[2] and b[1] and b[2]:
                    ar1,ar2=a[1]/a[2],b[1]/b[2]; ratio=abs(ar1-ar2)/max(ar1,ar2)
                if ratio<=0.08:
                    duplicate_rows.append({"finding_type":"matching normalized filename and similar aspect ratio (review visually)","file_a":a[0].removeprefix("File:"),"file_b":b[0].removeprefix("File:"),"geographic_coverage":area,"evidence":f"shared normalized name key “{key}”; relative aspect-ratio difference {ratio:.1%}"})
    links=defaultdict(set)
    for r in duplicate_rows: links[r["file_a"]].add(r["file_b"]); links[r["file_b"]].add(r["file_a"])
    for r in rows: r["duplicate_review_candidates"]=" | ".join(sorted(links[r["file"]],key=str.casefold))

    outdir.mkdir(parents=True,exist_ok=True)
    csvout(outdir/"maps.csv",rows,["file","geographic_coverage","short_description","matched_descendant_branches","commons_categories","usage_count_wikimedia_pages","usage_count_basis","mime_type","width_px","height_px","file_size_bytes","sha1","thumbnail_probe","thumbnail_http_status","thumbnail_content_type","possible_render_issue","duplicate_review_candidates","commons_file_page","global_usage_page"])
    csvout(outdir/"duplicate-candidates.csv",duplicate_rows,["finding_type","file_a","file_b","geographic_coverage","evidence"])
    csvout(outdir/"possible-render-issues.csv",issues,["file","geographic_coverage","possible_issue","evidence_categories","mime_type","thumbnail_http_status","commons_file_page"])
    groups=sum(1 for v in hashes.values() if len(v)>1)
    category_url="https://commons.wikimedia.org/wiki/"+urllib.parse.quote(args.root_category.replace(" ","_"),safe=":_")
    cache_files=list((outdir/"cache").glob("metadata-*.json"))
    if cache_files:
        times=[p.stat().st_mtime for p in cache_files]
        cache_start=datetime.fromtimestamp(min(times),timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
        cache_end=datetime.fromtimestamp(max(times),timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    else:
        cache_start=cache_end=datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    files_used=sum(1 for r in rows if r["usage_count_wikimedia_pages"]>0)
    total_usage_pages=sum(r["usage_count_wikimedia_pages"] for r in rows)
    leaders=sorted(rows,key=lambda r:r["usage_count_wikimedia_pages"],reverse=True)[:10]
    leader_lines=[]
    for r in leaders:
        name=r["file"].replace("|","\\|").replace("[","\\[").replace("]","\\]")
        leader_lines.append("| [{}](<{}>) | {:,} |".format(name,r["commons_file_page"],r["usage_count_wikimedia_pages"]))
    leader_table="\n".join(leader_lines)
    report=f"""# Wikimedia Commons SVG blank maps catalog

Catalog source: [{args.root_category}]({category_url}). Metadata batch checkpoints span **{cache_start}** to **{cache_end}** (UTC); the report was generated after the crawl.

- Recursive category-search branches: **{branch_count:,}**
- Unique file pages after deduplication: **{len(rows):,}**
- Files with at least one usage page: **{files_used:,}**
- Total distinct wiki/page usages across these files: **{total_usage_pages:,}**
- Potential SVG/render issues: **{len(issues):,}**
- Identical source-file SHA-1 groups: **{groups:,}**
- Duplicate-review pairs, including filename-based candidates: **{len(duplicate_rows):,}**

## Files

See maps.csv for one row per unique Commons file. Coverage and descriptions use Commons file descriptions and category names when available; ambiguous cases are marked for review. Each row links to the Commons file page and its GlobalUsage page.

## Most-used files

| Map | Distinct usage pages |
|---|---:|
{leader_table}

## Usage counts

usage_count_wikimedia_pages counts distinct `(wiki, page ID)` pairs returned by the [MediaWiki GlobalUsage API](https://www.mediawiki.org/wiki/API:Globalusage) for that file. It counts pages that use the file across Wikimedia projects; repeated embeds on one page count once. Collection spans multiple API requests rather than one atomic snapshot, so values may change during the crawl and after it finishes.

## Duplicate review

duplicate-candidates.csv lists exact SHA-1 matches and filename/aspect-ratio candidates. Exact matches have identical source bytes. Filename-based matches are leads for visual review, not confirmed duplicates; files can differ by projection, borders, or administrative detail.

## Possible rendering issues

possible-render-issues.csv lists files whose Commons categories flag invalid SVG source or broken file links, along with missing image metadata, absent SVG preview URLs, and failed or non-image thumbnail responses. Category flags can coexist with a reachable preview and are evidence for review, not proof the map fails to render. HTTP probe failures may be transient. A reachable image preview does not validate geographic content or visual accuracy; `thumbnail_probe` records the direct HTTP result.

Recursive deep-category searches were partitioned across the country branch to stay within MediaWiki limits of 256 categories and five levels. Coverage chooses the most specific geographic category/name signal available, then falls back to the file title or description. `Blank map.svg` is labeled as a generic locator template because its SVG has no fixed geographic region or country boundaries. Coverage is a category/name signal, not a geospatial validation of each SVG.
"""
    if branch_warnings:
        report += "\n## Recursive-search warnings\n\n"
        report += "\n".join(f"- {warning}" for warning in branch_warnings) + "\n"
    else:
        report += "\n## Recursive-search warnings\n\nNone.\n"
    (outdir/"README.md").write_text(report,encoding="utf-8")
    print(f"Wrote {outdir/'maps.csv'} ({len(rows):,} rows)",flush=True)
    print(f"Wrote {outdir/'duplicate-candidates.csv'} ({len(duplicate_rows):,} pairs)",flush=True)
    print(f"Wrote {outdir/'possible-render-issues.csv'} ({len(issues):,} rows)",flush=True)

if __name__=="__main__":
    main()
