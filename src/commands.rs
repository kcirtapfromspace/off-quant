        if summary.len() > (if args.verbose { std::usize::MAX } else { 200 }) {
+            break;
         }
     }
     println!("Sessions Summary:\n{}
", summary);
