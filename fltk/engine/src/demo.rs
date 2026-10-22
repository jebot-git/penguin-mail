// SPDX-License-Identifier: GPL-3.0-or-later
//! Sample mail uses upstream fake servers and the real sync and action paths.
use super::*;
use mailrs_domain::calendar::{Access, Calendar, Event};
use mailrs_sync::{
    AccountServices, AccountSync,
    fake::{FakeGmail, fill_store, meta},
};
pub async fn seed(db: &Db, engine: &Arc<SyncEngine>) -> Result<()> {
    let now = now_millis();
    for (i, email) in [
        "alex@example.com",
        "alex@studio.example",
        "alex@university.example",
    ]
    .iter()
    .enumerate()
    {
        let email = email.to_string();
        let email2 = email.clone();
        let id = db
            .write(move |c| accounts::insert_account(c, &email2, now))
            .await?;
        let fake = Arc::new(FakeGmail::new());
        fake.with(|s| {
            s.email=email.clone(); s.personal=true;
            s.sent_copy=Some(Box::new(move |raw|mailrs_sync::fake::read_sent(raw,id)));
            s.calendars.push(Calendar { id:email.clone(),name:email.clone(),color:"#568bbb".into(),access:Access::Owner,zone:"Europe/Prague".into(),primary:true,shown:true,hidden:false,reminders:vec![],all_day_reminders:None });
            s.calendar_events.push(Event { calendar:email.clone(),id:format!("meeting-{i}"),uid:format!("meeting-{i}@example.com"),title:["Design review","Lunch with Sam","Study group"][i].into(),start:now+3600000*(i as i64+1),end:now+3600000*(i as i64+2),busy:true,..Default::default() });
            s.contacts.push(mailrs_gmail::Person { resource:format!("people/demo-{i}"),name:Some(["Sam Park","Robin Chen","Morgan Reed"][i].into()),emails:vec![format!("friend{i}@example.net")],..Default::default() });
            let samples=[
                ("Welcome to Penguin Mail","Your mail, on your desktop","Welcome to Penguin Mail.\n\nThis FLTK edition uses the original Penguin Mail sync engine and native DomeD menus, fonts and tray integration.\n\nThis demo keeps all mail in a temporary database. You can archive, undo, reply, save drafts and explore the calendar without contacting a server."),
                ("Project notes for Friday","The updated plan is ready","Hi Alex,\n\nThe project notes are ready for review. Can we meet on Friday to go through the remaining work?\n\nThanks,\nSam"),
                ("Weekend plans","A walk by the river?","Hello,\n\nThe weather looks good this weekend. How about a walk by the river on Saturday morning?\n\nRobin"),
                ("Your booking is confirmed","See you next week","Your booking has been confirmed.\n\nArrival: Monday\nDeparture: Thursday\n\nPlease keep this message for your records."),
            ];
            for (j,(subject,snippet,text)) in samples.iter().enumerate() {
                let mid=format!("demo-{i}-{j}");
                let tags=if j==0 {vec!["INBOX","UNREAD","STARRED"]}else{vec!["INBOX","UNREAD"]};
                let mut m=meta(&mid,&mid,now-(j as i64*3600000+i as i64*60000),&tags);
                m.account_id=id;m.subject=subject.to_string();m.snippet=snippet.to_string();
                m.from=Some(Address {name:Some(["Penguin Mail","Sam Park","Robin Chen","Travel Desk"][j].into()),email:format!("sender{j}@example.net")});
                m.to=vec![Address {name:Some("Alex".into()),email:email.clone()}];
                s.messages.insert(mid.clone(),m);
                s.bodies.insert(mid.clone(),mailrs_domain::MessageBody {text:Some(text.to_string()),html:None,attachments:vec![],list_unsubscribe:None,one_click_unsubscribe:false,calendar:None,provenance:Default::default(),protection:None,receipt_to:None,read_report:None});
                let raw=mail_builder::MessageBuilder::new().from(format!("sender{j}@example.net")).to(email.clone()).subject(subject.to_string()).text_body(text.to_string()).write_to_vec().unwrap();
                s.raws.insert(mid,raw);
            }
        });
        let services = AccountServices::google(fake);
        let (tx, _rx) = async_channel::bounded(100);
        let sync = AccountSync::new(id, services.clone(), db.clone(), tx);
        fill_store(&sync).await?;
        engine.start_account(id, services);
    }
    Ok(())
}
