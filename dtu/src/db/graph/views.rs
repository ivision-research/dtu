diesel::table! {
   vsmali_methods (id) {
       id -> Integer,
       class_id -> Integer,
       source -> Text,
       smali -> Text
   }
}
