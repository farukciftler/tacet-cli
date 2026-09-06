use tacet_tools::router::{score_intent, IntentProfile};
#[test]
fn dominant_profiles() {
    let irr = ["Hello","Thank you very much.","Who are you?","How are you today?",
      "Are you sending my data to the cloud?","I'm a bit tired today, feeling low.",
      "Can you recommend a good movie?","What is the capital of France?",
      "What is the largest planet in our solar system?","Goodbye, see you!",
      "You did a fantastic job, thanks!","Tell me more about your thoughts.",
      "Which is better, morning or evening workout?","I'm feeling bored, tell me a joke.",
      "I love nature and fresh air.","Selam, nasılsın?","Çok teşekkürler, harikaydı!",
      "Bugün biraz yorgunum ya","Sence sabah sporu mu akşam sporu mu daha iyi?",
      "Sen kimsin, ne iş yaparsın?","Benim verilerimi başkalarıyla paylaşıyor musun?",
      "Bana güzel bir kitap önerir misin?","İyi akşamlar, sonra görüşürüz!",
      "Tebrik ederim harika bir iş çıkardın"];
    println!("--- IRRELEVANCE dominant profiles");
    for m in irr {
        let d = score_intent(m).dominant();
        println!("{:8} WEB={:5}  {}", format!("{:?}", d), d == IntentProfile::Web, m);
    }
    println!("--- WEB CASES");
    for m in ["How much is the dollar today?","Find flight schedules from London to Paris",
              "What is the weather in Istanbul?","Read the latest entries from app.log",
              "Calculate 15% off $80","Remember where I parked my car"] {
        println!("{:10} {}", format!("{:?}", score_intent(m).dominant()), m);
    }
}
