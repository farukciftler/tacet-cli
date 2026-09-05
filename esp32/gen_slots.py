"""Labelled slot examples, TR + EN, from templates.

THE TEST SET IS NOT GENERATED HERE. It is the 36 human-written cases in
benchmarks/tasks/, so the phrasings this trains on and the phrasings it is
scored on come from different hands. A generator that also wrote the test would
measure how well a linear model memorises its own templates, which is a number
nobody needs.

So the templates below are deliberately NOT the benchmark sentences: different
verbs, different word order, different fillers.
"""
import json, random, sys, itertools

rng = random.Random(7)

CITY_TR = ["istanbul","ankara","izmir","bursa","antalya","eskişehir","konya","adana",
           "trabzon","gaziantep","mersin","kayseri","samsun","denizli"]
CITY_EN = ["madrid","london","paris","berlin","rome","vienna","lisbon","prague",
           "amsterdam","dublin","porto","krakow","oslo","athens"]

# (surface form, label) — several surfaces per label, both languages.
# LANGUAGE IS TAGGED, NOT INFERRED. Deciding it with `str.isascii()` looks
# tempting and is wrong: "hafta sonu" and "ucuz" are pure ASCII Turkish, so the
# filter returns an empty list and the generator dies — or worse, silently
# builds a set with no weekend examples in it.
AUD = {
 "family":  {"tr":["ailece","aile olarak","tüm aile","ailemle birlikte"],
             "en":["as a family","with the whole family","family friendly"]},
 "kids":    {"tr":["çocuklarla","çocuk için","çocuklu","yeğenlerimle"],
             "en":["with the kids","for children","kid friendly"]},
 "adults":  {"tr":["yetişkinler için","sadece yetişkin","yetişkinlere uygun"],
             "en":["for adults","adults only","grown ups only"]},
 "seniors": {"tr":["yaşlılar için","büyüklerimiz için","yaşlıya uygun"],
             "en":["for seniors","for the elderly","suitable for older people"]},
}
PRICE = {
 "free":    {"tr":["ücretsiz","bedava","parasız","para vermeden"],
             "en":["free","at no cost","free of charge"]},
 "cheap":   {"tr":["ucuz","uygun fiyatlı","hesaplı","cebe uygun"],
             "en":["cheap","on a budget","inexpensive"]},
 "premium": {"tr":["lüks","pahalı","üst segment","şık"],
             "en":["premium","upscale","high end"]},
}
WHEN = {
 "today":    {"tr":["bugün","bugünlük","bu akşam"],
              "en":["today","later today","this evening"]},
 "tomorrow": {"tr":["yarın","yarına","yarın akşam"],
              "en":["tomorrow","tomorrow evening","the day after today"]},
 "weekend":  {"tr":["hafta sonu","cumartesi pazar","haftasonuna"],
              "en":["this weekend","over the weekend","on saturday"]},
}
HEAD_TR = ["nereye gidilir","ne yapılır","gezilecek yer var mı","nerede takılınır",
           "hangi mekanlar var","nereleri önerirsin","ne var ne yok"]
HEAD_EN = ["where should I go","what is worth seeing","any places to check out",
           "what can I do","suggest somewhere","where to spend the day"]

# QUOTES ARE BUILT COMBINATORIALLY, not listed. A flat list of five quotes per
# label gave 40 unique intent examples after dedup — the generator looked like it
# produced 1200 and the label counts said otherwise. Counting the set after
# dedup is the check that caught it.
PAY_TR  = ["ödeyeceğim","göndereceğim","yatıracağım","havale edeceğim","aktaracağım","hallederim"]
PAY_EN  = ["I will pay","I'll send it","I will transfer it","I'll settle it","you will get it"]
WHEN_TR = ["cuma günü","ayın 20sinde","önümüzdeki hafta","maaşımı alınca","salı","ay sonunda",
           "yarın","perşembe sabahı","gelecek pazartesi","15 gün içinde"]
WHEN_EN = ["on Friday","on the 12th","next week","after payday","by month end","on Tuesday",
           "tomorrow","Thursday morning","next Monday","within two weeks"]
DISP_TR = ["böyle bir şey sipariş etmedim","bu tutar yanlış","ben bu hizmeti almadım",
           "sözleşmede böyle bir madde yok","bu borç bana ait değil","yanlış kişiyle görüşüyorsunuz",
           "faturayı kabul etmiyorum","bu rakam fahiş, kabul etmem"]
DISP_EN = ["I never ordered that","this amount is wrong","I did not receive the service",
           "there is no such clause","this debt is not mine","you have the wrong person",
           "I reject that invoice","I only owe half of it"]
PAID_TR = ["dün havale ettim","geçen ay ödedim","bankadan gönderdim","dekontu yolladım",
           "hesabınıza geçti","ödemesini yaptım zaten","kartla ödedim","3 gün önce kapattım"]
PAID_EN = ["the transfer cleared yesterday","I settled it last month","money left my account",
           "I sent the receipt","it is already in your account","I paid it by card",
           "that was closed three days ago","it went out on the 2nd"]
IRR_TR  = ["bayramın kutlu olsun","geçmiş olsun","hafta sonu görüşürüz","tebrikler",
           "selamlar nasılsın","iyi tatiller","kolay gelsin","yeni yılın kutlu olsun"]
IRR_EN  = ["congratulations on the new role","happy holidays","see you at the conference",
           "hope you feel better","great news","enjoy your break","good luck with the move",
           "thanks for the update"]
FRAME_TR = ["Şunu yazdı: '{q}'. Ne demek istiyor?", "Karşı taraf '{q}' demiş, bu ne anlama gelir?",
            "Gelen cevap: '{q}' — nasıl yorumlamalıyım?", "'{q}' yazmış. Bu mesajı sınıflandır."]
FRAME_EN = ["They replied: '{q}'. What is this?", "The message was '{q}' — what does it mean?",
            "Classify this reply: '{q}'", "I got '{q}'. How should I read it?"]

def _quotes(label, lang):
    if label == "promised_date":
        pay  = PAY_TR if lang=="tr" else PAY_EN
        when = WHEN_TR if lang=="tr" else WHEN_EN
        return [f"{w} {p}" if lang=="tr" else f"{p} {w}" for p in pay for w in when]
    return {"dispute": DISP_TR if lang=="tr" else DISP_EN,
            "paid":    PAID_TR if lang=="tr" else PAID_EN,
            "irrelevant": IRR_TR if lang=="tr" else IRR_EN}[label]

CHAT_TR_A = ["sağ ol","teşekkürler","merhaba","günaydın","selam","iyi akşamlar","peki"]
CHAT_TR_B = ["yeterli bu kadar","çok yardımcı oldun","nasıl gidiyor","sen kimsin",
             "bugün canım sıkkın","bana bir şey anlat","ne düşünüyorsun","hangi dilleri biliyorsun",
             "biraz sohbet edelim","bugün hava nasıl sence","kendini nasıl tanımlarsın",
             "bir şiir yazar mısın","en sevdiğin renk ne","yorulmuyor musun hiç",
             "bugün ne yaptın","şaka yapmayı bilir misin","seninle konuşmak güzel",
             "akşam ne yesem acaba","müzik dinler misin","tatil önerin var mı hiç"]
CHAT_EN_A = ["thanks","hello","good morning","hey","cheers","hi there","alright"]
CHAT_EN_B = ["that helps","who made you","how are you doing","tell me something interesting",
             "I am not sure what to do","do you like music","what is your opinion on remote work",
             "just checking if you are there","let us chat a bit","what do you think about that",
             "how would you describe yourself","can you write a poem","what is your favourite colour",
             "do you ever get tired","what did you do today","can you tell a joke",
             "it is nice talking to you","what should I cook tonight","do you read books",
             "any thoughts on the weather"]

def search_examples(n):
    out=[]
    for _ in range(n):
        tr = rng.random() < 0.5
        city = rng.choice(CITY_TR if tr else CITY_EN)
        aud   = rng.choice([None]+list(AUD))
        price = rng.choice([None]+list(PRICE))
        when  = rng.choice([None]+list(WHEN))
        lang = "tr" if tr else "en"
        bits=[]
        if aud:   bits.append(rng.choice(AUD[aud][lang]))
        if price: bits.append(rng.choice(PRICE[price][lang]))
        if when:  bits.append(rng.choice(WHEN[when][lang]))
        rng.shuffle(bits)
        head = rng.choice(HEAD_TR if tr else HEAD_EN)
        if tr:
            msg = f"{city}da {' '.join(bits)} {head}".replace("  "," ").strip()
        else:
            msg = f"{head} in {city} {' '.join(bits)}".replace("  "," ").strip()
        out.append({"text":msg,"gate":"tool","tool":"search_filter",
                    "audience":aud or "none","price":price or "none",
                    "when":when or "none","intent":"none"})
    return out

def intent_examples(n):
    out=[]
    for _ in range(n):
        label = rng.choice(["promised_date","dispute","paid","irrelevant"])
        lang  = rng.choice(["tr","en"])
        frame = rng.choice(FRAME_TR if lang=="tr" else FRAME_EN)
        out.append({"text":frame.format(q=rng.choice(_quotes(label,lang))),"gate":"tool",
                    "tool":"message_intent","audience":"none","price":"none",
                    "when":"none","intent":label})
    return out

def chat_examples(n):
    out=[]
    for _ in range(n):
        tr = rng.random() < 0.5
        a, b = (CHAT_TR_A, CHAT_TR_B) if tr else (CHAT_EN_A, CHAT_EN_B)
        # The lead-in is optional: half of real small talk has no "thanks," in
        # front of it, and a gate that only ever saw the prefixed form would key
        # on the comma.
        t = f"{rng.choice(a)}, {rng.choice(b)}" if rng.random() < 0.6 else rng.choice(b)
        out.append({"text":t,"gate":"none","tool":"none","audience":"none",
                    "price":"none","when":"none","intent":"none"})
    return out

if __name__ == "__main__":
    n = int(sys.argv[1]) if len(sys.argv)>1 else 1200
    rows = search_examples(n) + intent_examples(n) + chat_examples(n)
    rng.shuffle(rows)
    seen=set(); uniq=[]
    for r in rows:
        if r["text"] in seen: continue
        seen.add(r["text"]); uniq.append(r)
    with open(sys.argv[2],"w",encoding="utf-8") as f:
        for r in uniq: f.write(json.dumps(r,ensure_ascii=False)+"\n")
    print(f"{len(uniq)} unique examples")
