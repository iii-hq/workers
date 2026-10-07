"""Regenerate lang.json: laya's own `analyse` over the cases below.

    git -C <laya clone> show v0.3.28:laya/lang.py > /tmp/laya_lang.py
    python3 -I tests/fixtures/lang.py /tmp/laya_lang.py

laya's lang.py has no dependencies, so it is loaded from that file alone (the
laya package's __init__ pulls in torch). Object keys are sorted before analysis, as
the iii bus delivers them. Character classes follow Python's Unicode version;
the fixture was made with Python 3.14 (Unicode 16.0).
"""
import importlib.util
import json
import sys
import unicodedata
from pathlib import Path

LONG_EN = " ".join(
    ["The customer reported that the dashboard loads slowly after the latest release, "
     "and the support team is collecting browser logs to share with engineering."] * 36)
DE = "Die Rechnung ist falsch und ich will mein Geld zurück"
FORM = {"subject": "New ticket from the web form", "body": "Quero cancelar meu plano",
        "channel": "web", "priority": "normal"}
ES_FORM = {"asunto": "Factura duplicada", "prioridad": "alta",
           "mensaje": "Hola, me cobraron dos veces la misma factura y necesito el reembolso"}


def nested(depth, leaf):
    state = leaf
    for key in reversed("abcdefg"[:depth]):
        state = {key: state}
    return state


CASES = {
    # laya 0.3.5's cases
    "english": "The customer says the invoice was billed twice and asks for a refund before Friday.",
    "short_en": "ok thanks",
    "empty": "",
    "digits": "12345 !!! ???",
    "portuguese": "O cliente diz que a fatura foi cobrada duas vezes e pede o reembolso até "
                  "sexta-feira, não é possível esperar.",
    "spanish": "El cliente dice que la factura fue cobrada dos veces y pide un reembolso para el viernes.",
    "french": "Le client dit que la facture a été facturée deux fois et demande un remboursement pour vendredi.",
    "german": "Der Kunde sagt, die Rechnung wurde zweimal berechnet und bittet um eine Rückerstattung bis Freitag.",
    "romanian": "Clientul spune că factura a fost facturată de două ori și cere o rambursare până vineri.",
    "turkish_no_stop": "Müşteri faturanın iki kez kesildiğini söylüyor ve cuma gününe kadar iade istiyor.",
    "hindi": "ग्राहक का कहना है कि चालान दो बार भेजा गया और वह शुक्रवार से पहले धनवापसी चाहता है।",
    "japanese": "顧客は請求書が二重に発行されたと言い、金曜日までに返金を求めています。",
    "mixed_dict": {"subject": "Refund", "body": "Veuillez rembourser la facture en double, merci "
                   "beaucoup pour votre aide.", "n": 3, "tags": ["urgent", "facture"]},
    "nested_list": [{"text": "hola, necesito ayuda con la factura porque me cobraron dos veces"},
                    "y quiero el reembolso"],
    "html_like": "<p>The <b>report</b> is due</p>",
    # #207: a foreign line or field next to a longer English part
    "pt_form_subject": {"subject": FORM["subject"], "body": FORM["body"]},
    "pt_error_payload": {"descricao": "O pagamento não foi processado", "error": {
        "message": "Your card was declined. Please try again with a different card or contact "
                   "your bank for more details."}},
    "pt_traceback": "O sistema caiu de novo hoje de manhã, segue o log:\nTraceback (most recent "
                    "call last):\n  File \"app.py\", line 42, in handle\n    conn = db.connect()\n"
                    "ConnectionError: the connection to the database was refused by the server",
    "pt_form_json_pretty": json.dumps(FORM, indent=2),
    "pt_form_json_compact": json.dumps(FORM),
    "pt_line_in_en_text": "Customer wrote the following message today:\nOlá, não consigo acessar "
                          "minha conta desde ontem\nPlease advise on next steps for this account.",
    "pt_comment_in_code": "def total(items):\n    # soma os valores da fatura para o cliente\n"
                          "    return sum(items)",
    "en_acronym_line": "Radio bands for tonight:\nMON LA EST COM DES are listed below\n"
                       "The schedule changes every week",
    "en_slash_compounds": "Check the Nav/Com panel and the OS/2 drive at C:\\DOS\\mode\n"
                          "Then restart ESA/UN mirror servers before noon",
    "en_code_lines": "import os\nresult = os.path.join(base, name)\nvalue = round(el, 2)\n"
                     "if non_english: print(value)",
    "es_json_pretty": json.dumps(ES_FORM, indent=2, ensure_ascii=False),
    "es_json_pretty_ascii": json.dumps(ES_FORM, indent=2),
    "en_json_compact": json.dumps({"ticket": 123, "summary": "Customer cannot log in after "
                                   "password reset", "tags": ["login", "auth"]}),
    "pt_crlf_lines": "Hello team,\r\nO pagamento não foi processado ontem à noite\r\nThanks",
    "pt_unicode_spaces": "Hello team\nQuero\u00a0cancelar\u001fmeu plano agora mesmo",
    "es_conversation": [
        {"role": "user", "content": "Hi, I need help with my order"},
        {"role": "assistant", "content": "Sure, what is the order number?"},
        {"role": "user", "content": "Es el pedido 4521 y todavía no ha llegado a mi casa"}],
    "pt_list_of_strings": ["Please see below", "Quero cancelar meu plano agora mesmo por favor"],
    # #406: any leaf, also past the 4000-character window
    "de_after_long_en": {"a_description": LONG_EN, "message": DE},
    "pt_after_long_en": {"a_description": LONG_EN, "message": "Quero cancelar meu plano agora mesmo"},
    "han_after_long_en": {"a_description": LONG_EN, "message": "我的订单还没到，请帮我查一下"},
    "han_short_after_long_en": {"a_description": LONG_EN, "message": "我的订单还没到"},
    "de_after_long_en_as_string": json.dumps({"a_description": LONG_EN, "message": DE}),
    "de_after_long_note": {"note": LONG_EN, "msg": "Ich habe zweimal bezahlt"},
    "de_line_past_window": LONG_EN + "\n" + DE,
    "de_after_digits": {"a_digits": "1234567890 " * 450, "message": DE},
    "multi_leaf_en": {"subject": "Login issue", "body": "I cannot log in to the dashboard since "
                      "this morning", "status": "open", "assignee": "Maria"},
    "multi_leaf_es": {"subject": "Problema", "body": "No puedo entrar a mi cuenta desde ayer por la tarde",
                      "user": "jperez"},
    "name_field_only": {"name": "José", "comment": "Please call me back tomorrow about the order"},
    "accented_leaf_undecided": {"a_note": LONG_EN, "z_msg": "Zamówienie nie dotarło, proszę o pomoc"},
    "de_at_depth_6": {**nested(6, DE), "x": "Please check the invoice status today"},
    "de_at_depth_7": {**nested(7, DE), "x": "Please check the invoice status today"},
    # #178 / #202 / #130: accent-free Romance, pt-BR and German
    "it_ascii": "Il cliente e stato addebitato due volte e vuole un rimborso",
    "fr_ascii": "Je veux annuler mon abonnement et etre rembourse",
    "shared_words_only": "la factura e la o casa",
    "it_articulated": "la fattura nella cartella dei documenti",
    "fr_merci": "merci pour votre aide",
    "es_ascii": "Hola, necesito ayuda porque me cobraron dos veces y quiero que me devuelvan el dinero",
    "pt_br_voce": "Voce pode me mandar a nota fiscal?",
    "pt_br_bug_report": "Deu erro 500 no endpoint de login depois do update",
    "pt_br_no_accents": "Oi, preciso de ajuda pra cancelar minha assinatura, nao consigo acessar a conta",
    "pt_shouting": "QUERO CANCELAR MEU PLANO AGORA MESMO, NAO AGUENTO MAIS",
    "pt_decomposed_accents": unicodedata.normalize(
        "NFD", "Não consegui pagar a fatura, você pode ajudar?"),
    "de_ascii": "ich habe heute keine zeit",
    "de_in_was": "was ist in der box drin",
    "de_ascii_fuer": "Ich brauche eine neue Rechnung fuer meinen Vertrag bitte",
    "nl": "Ik heb de factuur nog niet ontvangen en wil graag weten wanneer die komt",
    "ro_vs_it": "Clientul a primit factura dar nu a plătit încă suma",
    # #350: one accented loanword or name in English
    "en_loanword_jose": "Send the invoice to José before Friday",
    "en_loanword_cafe": "Please send me the café menu today please",
    "en_loanword_resume": "Could you email me your résumé before the meeting",
    "en_loanword_zurich": "We visited Zürich last summer and loved it",
    # #460: collision words
    "collision_come": "Come one, come all",
    "collision_son": "My son, your son",
    "collision_do": "Do more, do less",
    "collision_care": "Care more, care less",
    "collision_plus": "add 45 to 87 plus 54 plus 43 plus 22",
    "de_der_twice": "reduzieren der helligkeit der lichter",
    # #179: identifiers
    "identifiers_dict": {"url": "github.com", "email": "user@acme.com"},
    "identifiers_string": "github.com acme.com",
    "identifiers_in_english": "see github.com/os/com and docs.os.com for the os.path com.da details",
    "identifiers_only": "os.path com.br da.org os.com do.it",
    "emails_versions": "Contact admin@example.org about v1.2.3 and the U.S.A. release notes",
    # #728: Swedish and the Nordic overlap
    "sv_kan_inte": "Kan inte logga in",
    "sv_jag_kan_inte": "jag kan inte logga in",
    "sv_ascii": "jag behover hjalp med losenord idag",
    "sv_accented": "Jag behöver hjälp med min faktura, den har inte kommit fram än",
    "sv_short_fragment": "glömt lösenord",
    "sv_short_ascii": "glomt losenord",
    "nordic_greeting": "hej tack",
    "da_overlap": "Hej, jeg kommer om lidt og vi ses",
    "da_sentence": "Jeg har ikke modtaget min pakke endnu, kan I hjælpe mig?",
    "da_ascii": "hej, jeg vil gerne have hjaelp med min ordre tak",
    # #113: non-Latin text with Latin brand names; annotation that is not
    "han_with_brand": {"body": "我的 iPhone 15 Pro Max 订单还没到"},
    "kana_with_brand": {"body": "Amazonで買ったiPhoneが届かない"},
    "korean_with_brand": "갤럭시 S24 Ultra 주문이 아직 안 왔어요",
    "hindi_with_brand": "Samsung Galaxy S24 Ultra Pro Max order नहीं आया",
    "en_greek_symbol": "Set α to 0.05 and rerun the test with the same seed",
    "en_cyrillic_name": "The ticket was filed by Дмитрий Петрович Савицкий yesterday",
    "en_cyrillic_combining_name": "The report was written by Влади́мир yesterday evening",
    "en_ipa": "The name Vladimir is pronounced [vlɐˈdʲimʲɪr] in Russian",
    "en_greek_word": "Please translate the word καλημέρα for the customer",
    "en_greek_words": "Please translate καλημέρα σας φίλε for the customer",
    "en_halfwidth_kana": "Order ｶｷｸｹｺ shipped",
    # #169: letters no range claims; fullwidth Latin
    "bopomofo": "ㄅㄆㄇㄈ ㄉㄊㄋㄌ ㄍㄎㄏ",
    "halfwidth_kana": "ｱｲｳｴｵ ｶｷｸｹｺ",
    "cjk_ext_b": "𠀀𠀁𠀂𠀃",
    "fullwidth_latin": "ＨＥＬＬＯ ＷＯＲＬＤ",
    # #36: Azerbaijani; dotted capital I
    "az": "Mən sizin xidmətinizdən razı deyiləm və pulumu geri istəyirəm",
    "az_dotted_i": "İndi mən və siz bu məsələni həll etməliyik",
    "tr_dotted_i": "İstanbul ofisindeki İnsan Kaynakları ekibi bu ayın maaşını henüz ödemedi",
    # #187: romanized Bangla
    "bn_romanized": "ami invoice er jonno duibar charge peyechi, refund chai",
    "bn_romanized_2": "ami tomake bhalobashi kintu amar kotha shono",
    # #203: undecided Latin text
    "pt_undecided_short": "Quero cancelar",
    "pt_undecided_dict": {"message": "Esqueci minha senha"},
    # other scripts, ties and Latin without a list
    "korean": "주문한 상품이 아직 도착하지 않았습니다",
    "arabic": "لم يصل طلبي حتى الآن، أرجو المساعدة",
    "hebrew": "ההזמנה שלי עדיין לא הגיעה",
    "thai": "สินค้าที่สั่งยังไม่มาถึง",
    "ethiopic": "ትዕዛዜ እስካሁን አልደረሰም",
    "georgian": "ჩემი შეკვეთა ჯერ არ მოსულა",
    "armenian": "Իմ պատվերը դեռ չի եկել",
    "tamil": "என் ஆர்டர் இன்னும் வரவில்லை",
    "cyrillic_greek_tie": "абв αβγ",
    "greek_cyrillic_tie": "αβγ абв",
    "latin_cyrillic_tie": "abc абв",
    "pl_no_list": "Zamówienie nie dotarło jeszcze, proszę o pomoc w tej sprawie",
    "vi": "Tôi chưa nhận được đơn hàng của mình",
    "emoji_en": "Thanks so much for the quick help 🙏🎉 you are the best",
    # rounding of the reported rates (1/160 is 0.0063 in Python)
    "rounding_diacritic_160": ("café " + "ok " * 60)[:160],
    "rounding_diacritic_800": ("café " + "ok " * 300)[:800],
    "rounding_non_latin_160": "α " + "abc " * 53,
    # Unicode versions: marks Unicode 9 lacks never split or lengthen a word;
    # `ʕ` is lowercase in Python 3.14 (Unicode 16), so its line is not all capitals
    "arabic_mark_after_unicode_9": {"msg": "ok \u0628\u0898\u062a", "name": "Ana"},
    "hangul_malayalam_mark": "Order ok: \uc844\u0d3b\ucc03",
    "telugu_nukta_after_unicode_9": "ok \u0c0e\u0c3c ok",
    "pt_shouting_with_pharyngeal": "Hello team, please see the note below.\n"
                                   "QUERO CANCELAR MEU PLANO \u0295",
    # states that are not text
    "null_state": None,
    "number_state": 42,
    "bool_state": True,
    "empty_dict": {},
    "empty_list": [],
}


def main():
    spec = importlib.util.spec_from_file_location("laya_lang", sys.argv[1])
    lang = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(lang)
    fixtures = {}
    for name, state in CASES.items():
        state = json.loads(json.dumps(state, sort_keys=True))
        fixtures[name] = {"input": state, "expected": lang.analyse(state)}
    out = Path(__file__).with_name("lang.json")
    out.write_text(json.dumps(fixtures, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
