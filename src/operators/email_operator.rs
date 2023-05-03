use crate::{
    data::models::{Invitation, PasswordReset},
    errors::DefaultError,
};
use sendgrid::v3::{Content, Email, Message, Personalization, Sender};

pub fn send_invitation(invitation: &Invitation) -> Result<(), DefaultError> {
    let sg_email_content = format!(
        "Please click on the link below to complete registration. <br/>
         <a href=\"htts://arguflow.com/auth/register/{}?email={}\">
         https://arguflow.com/register</a> <br>
         your Invitation expires at <strong>{}</strong>",
        invitation.id,
        invitation.email,
        invitation.expires_at.format("%I:%M %p %A, %-d %B, %C%y")
    );
    let sg_email_personalization = Personalization::new(Email::new(invitation.email.as_str()));
    let sg_email = Message::new(Email::new("no-reply@arguflow.com"))
        .set_subject("You have been invited to join Arguflow Editor")
        .add_content(
            Content::new()
                .set_content_type("text/html")
                .set_value(sg_email_content),
        )
        .add_personalization(sg_email_personalization);

    send_email(sg_email)
}

pub fn send_password_reset(password_reset: &PasswordReset) -> Result<(), DefaultError> {
    let sg_email_content = format!(
        "Please click on the link below to reset your password. <br/>
         <a href=\"htts://arguflow.com/auth/password/{}?email={}\">
         htts://arguflow.com/auth/password</a> <br>
         your password reset link expires at <strong>{}</strong>",
        password_reset.id,
        password_reset.email,
        password_reset
            .expires_at
            .format("%I:%M %p %A, %-d %B, %C%y")
    );
    let sg_email_personalization = Personalization::new(Email::new(password_reset.email.as_str()));
    let sg_email = Message::new(Email::new("no-reply@arguflow.com"))
        .set_subject("Reset your Arguflow Editor password")
        .add_content(
            Content::new()
                .set_content_type("text/html")
                .set_value(sg_email_content),
        )
        .add_personalization(sg_email_personalization);

    send_email(sg_email)
}

fn send_email(sg_email: Message) -> Result<(), DefaultError> {
    let sg_api_key = std::env::var("SENDGRID_API_KEY").expect("SENDGRID_API_KEY must be set");
    let sg_sender = Sender::new(sg_api_key);
    let sg_response = sg_sender.send(&sg_email);
    match sg_response {
        Ok(_) => {
            log::info!("Email sent successfully");
            Ok(())
        }
        Err(_e) => Err(DefaultError {
            message: "Error sending email.",
        }),
    }
}
