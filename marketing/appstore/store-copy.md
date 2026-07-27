# Tacet — App Store store copy

Version 1.0 · Every capability claim in this document was verified against the source code (verification notes in §10).

**Brand: Tacet.** The name change also changed the length of the copy — the old name was 4 characters, `Tacet` is 5. All the character counts in §1–§4 were **recounted** after the rename rather than estimated by hand; none exceeds its App Store limit.

The privacy frame in use: **the core runs on the device; web and connections engage only if you turn them on, and only in plain sight.** No text contains an absolute claim ("no data ever leaves the device", "fully offline").

> **Localisation note.** This app ships a Turkish interface localisation (`tr` is one of the
> eight locales in `Localizable.xcstrings`, whose source language is `en`). The strings in
> the `tr-TR` blocks below are therefore **shipping listing copy for that locale**, in the
> same class as the localised UI strings — not documentation. They are kept verbatim and
> their character counts are measurements of those exact strings. All editorial and
> instructional prose in this document is English.

---

## 1. App name (30 characters)

### tr-TR listing candidates

| Candidate | Characters | Note |
|---|---|---|
| **Tacet — cihazında asistan** | 25 | **Recommended.** Name + a single attribute; carries the "assistant" and "device" search signal. |
| Tacet — cihazda asistan | 23 | Shorter, weaker personal connection. |
| Tacet: cihaz üstü asistan | 25 | A colon instead of a dash; the dash reads more conventionally on the App Store. |

### en listing candidates

| Candidate | Characters | Note |
|---|---|---|
| **Tacet — on-device assistant** | 27 | **Recommended.** |
| Tacet — assistant on iPhone | 27 | Does not say that it runs on the device. |
| Tacet: private assistant | 24 | "private" compresses the claim into one word, and it cannot be proved. |

### Spelling rule — CHANGED

The name is **Tacet** everywhere: initial capital, the rest lower case. Do not write `TACET`,
`tacet` or `TaCet`. **The "the brand is lower case everywhere" rule from the previous version
is invalid** — that rule was for the old name, and applying it to the new one writes the name
field wrongly.

The lower-case `tacet` is valid only as the **command/binary name** (the desktop Rust shell);
none of the store copy uses that form.

In Turkish listing copy, suffixes take an apostrophe: Tacet'i, Tacet'e, Tacet'in, Tacet'te,
Tacet'ten.

### The story of the name (grounding for the copywriter; does not go into the store as-is)

**Tacet** is the term in musical notation announcing that **an instrument is silent** for a
passage (Latin *tacet*: it is silent). It also appears related to the English *tacit* — understood
without being said.

The name was chosen because it maps exactly onto the product's promise: "tacet" names
**something that is not done** — being silent is an action, and the silence is written into the
score. The product's claim is not "we keep your data safe" but "we do not talk unnecessarily":
no server, no account, network surfaces off by default. Silence is the default state here;
speaking is something the user turns on.

In the store copy the explanation of the name is **never used as a claim** — the adjective
"quiet" is carried by the product's behaviour, not by the name's etymology.

---

## 2. Subtitle (30 characters)

### tr-TR listing candidates

| Candidate | Characters |
|---|---|
| **Cihazında çalışan asistan** | 25 |
| Çekirdeği cihazda çalışır | 25 |
| Takvim, belge, hatırlatıcı | 26 |
| Sessiz, cihaz üstü asistan | 26 |
| Telefonunda çalışan asistan | 27 |

**Recommendation: "Cihazında çalışan asistan"** — it says what it is and where it runs in a
single breath, sets up no absolute claim, and repeats nothing from the name. "Çekirdeği
cihazda çalışır" is more honest-technical but says nothing about what the product does; it
loses the reader at first contact.

### en listing candidates

| Candidate | Characters |
|---|---|
| **On-device personal assistant** | 28 |
| Calendar, notes, documents | 26 |
| A quiet on-device assistant | 27 |
| Runs on your iPhone, not us | 27 |
| The core runs on your phone | 27 |

**Recommendation: "On-device personal assistant".**

---

## 3. Promotional text (170 characters)

**tr-TR (164)**

> Tacet takvimine bakar, hatırlatıcı kurar, notlarında arar, belge üretir. Çekirdeği iPhone'unda çalışır; web araması ve bağlantılar yalnız sen açarsan devreye girer.

**en (168)**

> Tacet reads your calendar, sets reminders, searches notes and builds documents. The core runs on your iPhone; web search and connections start only if you turn them on.

---

## 4. Description (4000 characters)

### tr-TR (2 949 characters — limit 4000)

```
Tacet, iPhone'unda çalışan kişisel bir asistandır. Takvimine bakar, hatırlatıcı kurar, notlarında arar, belge üretir.

Çekirdeği cihazında çalışır. Web araması ve bağlantılar yalnız sen açarsan, sen görerek devreye girer.

ÇEKİRDEK CİHAZDA
Yanıtları Apple Intelligence'ın cihaz üstü modeli üretir. Sohbetlerin, takvimin, kişilerin ve notların bu iş için telefonundan çıkmaz. Tacet'in sunucusu yok: hesap açmazsın, giriş yapmazsın, reklam ve ölçüm izleyicisi taşımaz.

Cihaz dışına çıkan iki yüzey var, ikisi de varsayılan kapalı:
· Web araması — kendi arama sunucunun adresini girersen Tacet web'de arar.
· Bağlantılar — kendi MCP sunucunu eklersen onun araçlarını kullanır.
Kutudan hazır adres gelmez. İkisini de sen açarsın, sorgunun gittiği yeri sen belirlersin.

NELER YAPAR
· Takvim — "Yarın neler var?" Etkinlikleri okur, yenisini ekler.
· Hatırlatıcılar — hatırlatıcı kurar, bekleyenleri listeler.
· Kişiler — rehberden numara ve e-posta bulur.
· Notlar ve dosyalar — cihazındakiler arasında arar.
· Belge üretir — Excel, Word, PDF, Markdown, düz metin, tek sayfalık web sayfası.
· Belge okur — eklediğin PDF, Word, Excel, Markdown ya da metin dosyasını özetler, içinden soruya cevap verir.
· Belge düzenler — ürettiği dosyaya satır ekler, çıkarır, başlığını değiştirir.
· Hesap ve zaman — aritmetiği kafadan yapmaz, araçla yapar; iki tarih arasını sayar.
· Kod — ağı ve dosya sistemi olmayan kapalı bir kutuda kısa JavaScript çalıştırır.
· Hafıza — sohbetlerinden kalıcı notlar çıkarır, sonraki sohbette hatırlar. Hepsini görür, istediğini silersin.
· Beceri — kendi çalışma kuralını yazarsın; tetikleyici kelimen geçtiğinde Tacet onu okur.
· Sesle yaz — mikrofona bas, söylediklerin bu cihazda yazıya döner. Metni göndermeden görürsün.
· Siri ve Kısayollar — soru sor, belge ürettir, hafızaya not ekle, üretilen dosyayı al.

GÖRÜNÜR ARAÇLAR
Tacet bir araca her dokunduğunda akışta iz kalır: "Takvim okundu · yarın". İze dokununca aracın ham girdisini ve çıktısını görürsün. Cihaz dışına giden bir çağrıda gönderilen içerik de aynı yerde durur. Kişisel verine dokunulmuş bir sohbette dışarı çıkan çağrı, sen onaylamadan gitmez.

DÜRÜST SINIRLAR
· Model küçük. Genel dünya bilgisi zayıf; emin olmadığında uydurmaz, bilmediğini söyler.
· Uzun konuşmalarda bağlamı taşıyamayabilir; yeni sohbet çoğu zaman çözer.
· Görsel üretmez, sesli yanıt vermez, e-posta ya da mesaj göndermez.
· Hava durumu ve harita servisi yok.
· Yalnız iPhone, yalnız dikey düzen.

GEREKSİNİMLER
· iOS 26 ve Apple Intelligence destekleyen bir iPhone (iPhone 15 Pro ve sonrası).
· Apple Intelligence açık olmalı: Ayarlar > Apple Intelligence ve Siri. Kapalıyken Tacet yanıt üretemez ve bunu ekranda söyler.
· Takvim, hatırlatıcı, kişiler ve mikrofon izni önden toplanmaz; her biri ilk gerektiğinde sorulur. Vermezsen kalan her şey çalışır.

Arayüz dilleri: Türkçe, İngilizce, Almanca, İspanyolca, Fransızca, Japonca, Korece, Portekizce (Brezilya), Basitleştirilmiş Çince.
```

### en (3 106 characters — limit 4000)

```
Tacet is a personal assistant that runs on your iPhone. It reads your calendar, sets reminders, searches your notes and builds documents.

The core runs on your device. Web search and connections start only if you turn them on, and only where you can see them.

THE CORE STAYS ON THE DEVICE
Answers are produced by Apple Intelligence's on-device model. Your chats, calendar, contacts and notes are not sent anywhere for this. Tacet has no server: no account, no sign-in, no ad or analytics tracker.

Two surfaces can leave the device, and both are off by default:
· Web search — enter the address of your own search server and Tacet searches the web.
· Connections — add your own MCP server and Tacet can use its tools.
No address ships with the app. You turn both on, and you decide where a query goes.

WHAT IT DOES
· Calendar — "What's on tomorrow?" Reads events, adds new ones.
· Reminders — creates reminders, lists the pending ones.
· Contacts — finds a number or an email address.
· Notes and files — searches what is already on your device.
· Builds documents — Excel, Word, PDF, Markdown, plain text, single-page website.
· Reads documents — attach a PDF, Word, Excel, Markdown or text file; it summarises and answers questions about it.
· Edits documents — adds rows, removes them, changes the title, writes a new version.
· Maths and time — arithmetic is done by a tool, never in the model's head; counts days between dates.
· Code — runs short JavaScript in a sandbox with no network and no file system.
· Memory — keeps lasting notes from your chats and recalls them later. You see every note and delete what you want.
· Skills — write your own working rule; Tacet reads it when your trigger word appears.
· Dictation — tap the microphone; speech becomes text on this device. You read it before sending.
· Siri and Shortcuts — ask a question, generate a document, add a note, hand over the last file.

TOOLS YOU CAN SEE
Every time Tacet touches a tool it leaves a trace in the thread: "Calendar read · tomorrow". Tap the trace and you see the tool's raw input and output. For a call that leaves the device, the content sent sits in the same place. In a chat where personal data has been touched, nothing leaves without your approval.

HONEST LIMITS
· The model is small. General world knowledge is weak; it says it does not know instead of inventing.
· It may lose context in long conversations; a new chat usually fixes that.
· No image generation, no spoken replies, no sending email or messages.
· No weather or maps service.
· iPhone only, portrait only.

REQUIREMENTS
· iOS 26 and an iPhone that supports Apple Intelligence (iPhone 15 Pro or later).
· Apple Intelligence must be on: Settings > Apple Intelligence & Siri. While it is off Tacet cannot answer, and it says so on screen.
· Calendar, reminders, contacts and microphone permissions are never collected up front; each is asked for the first time it is needed. Decline one and everything else still works.

Interface languages: Turkish, English, German, Spanish, French, Japanese, Korean, Portuguese (Brazil), Simplified Chinese.
```

---

## 5. Keywords (100 characters, comma-separated, no spaces)

Rule: do not repeat words from the name and subtitle (Apple already indexes those), use the singular, add no spaces.

**tr-TR (92)**

```
yapay zeka,gizlilik,yerel,takvim,hatırlatıcı,not,arama,excel,word,pdf,belge,mcp,siri,kısayol
```

**en (99)**

```
ai,private,local,llm,calendar,reminder,notes,search,excel,word,pdf,document,summarise,mcp,shortcuts
```

**Change (audit round): the words for "offline" were REMOVED from both locales.** The previous version carried them on the grounds that "the keyword field is invisible, it is only for matching". That rationale is not good enough:

- Apple **reviews** keywords; the field being invisible to the user does not stop it from being a claim. The updated App Review Guideline 2.3 (Accurate Metadata) targets metadata directly.
- As long as the app carries the web search and MCP surfaces, "offline" is an absolute characterisation; the brand rule forbids an absolute claim regardless of how visible the text is.
- "local" covers the same search intent and is accurate: the core inference is done on the device.

"private" was kept in EN — it is not absolute, it is a qualitative positioning adjective in this market, and the description backs it with concrete behaviour (no server, no account, network surfaces off by default).

---

## 6. Release note / What's New (1.0)

**tr-TR**

```
İlk sürüm.

· Takvim, hatırlatıcı, kişiler ve cihazdaki notlarda arama.
· Excel, Word, PDF, Markdown, metin ve tek sayfalık web sayfası üretimi; eklediğin belgeyi okuma ve düzenleme.
· Hesap, tarih hesabı ve kapalı kutuda JavaScript.
· Hafıza ve kendi yazdığın beceriler.
· Cihaz üstü sesle yazma.
· Siri ve Kısayollar: sor, belge üret, not ekle, son belgeyi al.
· Web araması ve bağlantılar — varsayılan kapalı, kendi sunucunla.
· Dokuz dil, koyu mod.
```

**en**

```
First release.

· Calendar, reminders, contacts and search across the notes and files on your device.
· Builds Excel, Word, PDF, Markdown, text and single-page web documents; reads and edits the ones you attach.
· Maths, date arithmetic and JavaScript in a closed sandbox.
· Memory and skills you write yourself.
· On-device dictation.
· Siri and Shortcuts: ask, generate a document, add a note, hand over the last file.
· Web search and connections — off by default, with your own server.
· Nine languages, dark mode.
```

---

## 7. URLs (placeholders — to be filled before release)

| Field | Value | State |
|---|---|---|
| Support URL | `https://<domain>/tacet/support` | **PLACEHOLDER** — mandatory, must be a working page |
| Marketing URL | `https://<domain>/tacet` | optional |
| Privacy policy URL | `https://<domain>/tacet/privacy` | **PLACEHOLDER** — mandatory |
| Support email (App Review) | `<name>@<domain>` | **PLACEHOLDER** |

The support page needs at minimum: what it is, a contact route, the Apple Intelligence requirement, and an FAQ (why the microphone is inactive, why the model does not answer, how to turn on web search).

### What the privacy policy must contain (for the user to write)

1. **Who.** The publisher's name and contact address. That Tacet has no server and that there is no publisher infrastructure the app could send data to.
2. **Where processing happens.** That model inference, chat history, calendar/reminder/contact/note data and document production are on the device, and that no network is used for these.
3. **What is stored, and where.** Chats, memory notes, skills and settings on the device (SwiftData); generated and attached documents in the device's Documents folder. That these may be included in a device backup and that the user can turn that off. That chat history can be deleted from Settings, notes from the Memory board, and files from Settings > Documents.
4. **When web search is on.** That the search query goes to the server address **the user entered themselves**; that the app has no embedded default address, no API key and no publisher-owned search server; that the logging policy of that server belongs to the server's owner (i.e. usually the user). That the default is off.
5. **When connections (MCP) are on.** That the tool name and tool arguments go to the MCP server the user chose; that which data goes depends on the user's request and on that server's tools; that it is shown on screen and approved before being sent; that connection keys are kept in the device Keychain, on this device only and accessible only while unlocked.
6. **System services.** That speech recognition is done on-device and never falls back to server recognition; that the audio recording is not written to disk. That crash reports go to Apple only through Apple's own mechanism and with the user's consent.
7. **What is not collected.** No analytics SDK, no advertising identifier, no tracking, no cookies, no fingerprinting. No account, no sign-in, no email collected.
8. **The Shortcuts chain.** That while "hand files to Shortcuts" under Settings > Shortcuts is on, a generated file can be handed to the user's own automation, and that what happens after that is the user's decision. That the default is off.
9. **Children / age.** That the app is not directed at children and collects no data from children.
10. **Effective date and changes.** The date and how the policy is updated.

---

## 8. App Store privacy label — draft answers

Verified in the code: no analytics/tracking SDK, no account/sign-in, `URLSession` in only two files (`Services/MCPClient.swift`, `Services/WebSearchClient.swift`), no remote server belonging to the app, and `PrivacyInfo.xcprivacy` declares `NSPrivacyTracking=false` with an empty `NSPrivacyCollectedDataTypes`.

| Question | Answer |
|---|---|
| Does this app collect data? | **No — "Data Not Collected"** |
| Is there tracking? | No. `NSPrivacyTracking = false`, no tracking domain, ATT is never requested |
| Third-party ads / analytics | None |
| Account creation / sign-in | None |
| Purchases, subscriptions | None |
| Required reason APIs (manifest) | UserDefaults `CA92.1`; FileTimestamp `C617.1`, `3B52.1`; SystemBootTime `35F9.1` |

**Why "Data Not Collected" is defensible:** in Apple's definition, collection is data leaving the device and becoming accessible to *the developer or their third party*. In Tacet, data never goes to the developer under any circumstances — there is no server for it to go to. Network traffic leaves only to the address the user entered themselves, over a surface the user turned on, with an approval the user saw; the receiving endpoint is the user. This must be explained explicitly in the App Review notes (§9) and written into the privacy policy.

**A fallback declaration prepared in case the reviewer objects** (only if needed; do not volunteer it):

| Data type | Use | Linked to identity | For tracking |
|---|---|---|---|
| Search History (the search query) | App Functionality | No | No |
| Other User Content (tool arguments) | App Functionality | No | No |

If this fallback declaration is made, the label must be accompanied by the explanation "to a server the user configured themselves, when the user turns it on".

**What must not be done:** ticking "Data Not Collected" and then never mentioning the network surfaces in the policy; or declaring a surface that is off by default as if it "always sends data". Both are claim-behaviour inconsistencies.

---

## 9. App Review notes

The text below goes into App Store Connect > App Review Information > Notes. **Critical:** the app cannot produce an answer while Apple Intelligence is off; if the reviewer does not know this they may reject it as "does not work / empty app".

```
No account is required, there is no sign-in, there are no purchases. A demo account is not needed.

1. DEVICE AND MODEL REQUIREMENT — PLEASE READ FIRST
Tacet produces answers with Apple Intelligence's on-device model (the Foundation
Models framework, SystemLanguageModel). The app has no server or cloud model of
its own; Private Cloud Compute is not used. For this reason the chat produces no
answer on a device where Apple Intelligence is off or unsupported.

Requirements:
· An iPhone that supports Apple Intelligence (iPhone 15 Pro or later).
  The model cannot be used in the simulator or on an unsupported device.
· iOS 26 or later.
· The device language and region must be one that Apple Intelligence supports.

Steps to turn it on:
1. Settings > Apple Intelligence & Siri.
2. Turn Apple Intelligence on.
3. Wait for the model download to finish (Wi-Fi is required, it can take a few
   minutes). While the download is running Tacet says "preparing the model" on screen.
4. Open Tacet and type something like "What's on tomorrow?" into the input field
   at the bottom.

If the model is unavailable the app does not crash: it states the situation on
screen in plain text (preparing the model / Apple Intelligence off / device
unsupported). It is not a blank screen or a silent failure.

2. WHEN NETWORK USE COMES INTO PLAY
In the default setup the app makes no network requests at all. There are only two
optional surfaces that go online, and BOTH ARE OFF BY DEFAULT:

· Web search: Settings > WEB SEARCH. The user enters the address of their own
  SearXNG server, verifies it with "Test the server", then turns it on. There is
  NO embedded default address, API key or publisher-owned server in the app — until
  an address is entered, the search tool never enters the session at all.
· Connections (MCP): Settings > CONNECTIONS. The user adds their own MCP server.
  Likewise there is no ready-made server list.

The network code is in only two files: Services/WebSearchClient.swift and
Services/MCPClient.swift. The model layer, the tool layer and the personal-data
tools have no network access.

If one of the personal-data tools (calendar, reminder, contacts, device search,
document) actually ran in that chat, every call leaving the device first stops at
an approval screen and the exact arguments to be sent are shown to the user. Even
if the approval is skipped, the content sent appears in raw form in the tool trace.

You do not have to test these surfaces; all the core functions of the app work
when they are left off. If you would like to try, you can enter the address of a
public SearXNG instance (its JSON format must be enabled) into Settings > WEB
SEARCH and press "Test the server"; no publisher test server is provided, because
by design the product has no such server.

3. PERMISSIONS
Calendar, Reminders, Contacts, Microphone and Speech Recognition permissions are
not requested up front in a batch; each is requested the first time the relevant
tool actually needs it. The app is usable without granting any of them (document
production, maths, time, code, memory and skills all work).

Speech recognition is on-device and never falls back to server recognition. If the
on-device language model is not ready the microphone button stays disabled and
states the reason; this is deliberate behaviour, not a bug.

4. SIRI AND SHORTCUTS
Four shortcuts are exposed: ask a question, generate a document, add a note to
memory, hand over the most recently generated document. The file output in the
last item is protected by a separate switch under Settings > SHORTCUTS and is OFF
by default. Web search and MCP tool calls are deliberately not opened to
Shortcuts: the approval screen requires a visible interface.

5. ABOUT CONTENT GENERATION
The model is small and its world knowledge is limited; it is steered towards not
inventing when it does not know. There is no user-generated shared content, chat
network, comment system or any other channel reaching another user inside the app.

6. CONFIGURATION
iPhone only, portrait only. The interface is in nine languages. No ads, no
analytics, no third-party SDK, no accounts.
```

---

## 10. Verification notes (does not go into the store)

The source of every claim:

| Claim | Source |
|---|---|
| On-device model, no PCC | `Tacet/Services/ModelService.swift` (`SystemLanguageModel.default`), spec §7.1 |
| Network in only two files | `grep -rn URLSession Tacet/` → only `Services/MCPClient.swift`, `Services/WebSearchClient.swift` |
| No analytics / tracking | grep for analytics, firebase, sentry, crashlytics, amplitude, mixpanel, posthog, ATT → no results |
| No account / sign-in | grep for `signIn`, `createAccount`, `AuthenticationServices`, `StoreKit` → no results |
| The tool catalogue | `Tacet/Tools/`: Calendar, Reminder, Contact, SearchNotes, CreateDocument/Read/Edit, Calc, RunCode, Time, WebSearch, MCP |
| Document formats (writing) | `DocumentEngine.engine`: xlsx, pdf, docx, md, txt, html |
| Document formats (reading/attaching) | `Views/ChatView.swift` `documentTypes`: pdf, plainText, text, xlsx, docx, md |
| Code sandbox (no network/files) | `Tools/CodeEngine.swift` — a fresh `JSVirtualMachine`, a 3 s timeout |
| The memory and skill boards | `Views/MemoryBoard.swift`, `Views/SkillBoard.swift` |
| Tool trace + raw input/output | `Views/ToolChip.swift`, `Services/TimelineRecorder.swift`, spec §4.4 |
| The approval gate / dirty session | `Services/ToolExecutor.swift`, spec §7.5 |
| On-device dictation | `Services/VoiceInput.swift` (`SpeechTranscriber`, `requiresOnDeviceRecognition`) |
| Four App Shortcuts | `Tacet/Intents/TacetShortcuts.swift` |
| Handing files to Shortcuts is off by default | `Intents/ShortcutSetting.swift` `exportKey`, `@AppStorage ... = false` |
| Web search off by default, no embedded address | the web section of `Views/Settings.swift` + `Services/WebSearchSetting.swift` |
| Keys in the Keychain, never leaving the device | `Services/Keychain.swift` (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`) |
| iOS 26, iPhone only, portrait | `Tacet.xcodeproj/project.pbxproj`: `IPHONEOS_DEPLOYMENT_TARGET = 26.0`, `TARGETED_DEVICE_FAMILY = 1`, `UISupportedInterfaceOrientations_iPhone = Portrait` |
| Nine languages | `Tacet/Localizable.xcstrings`: en (source), de, es, fr, ja, ko, pt-BR, tr, zh-Hans |
| The privacy manifest | `Tacet/PrivacyInfo.xcprivacy` |

**Not written, because it could not be verified:** widget, lock screen, spoken replies, image generation, iPad, macOS, cloud backup, scheduled briefing (the "watch" tool was dropped from v1), weather, maps.
